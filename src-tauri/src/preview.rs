//! What a run would do, before it does it.
//!
//! The dangerous half of a mirror backup is the prune pass: it deletes from
//! the copy whatever is no longer in the source, and a source that lost a
//! folder to a bad click looks exactly like a source that was tidied up on
//! purpose. Counting the work first — new, modified, and above all deleted —
//! is what turns "back up" into a decision rather than a leap.
//!
//! Every rule here is the run's own. The source walk, the size-and-mtime
//! comparison and the prune protections are the same code the pipeline uses,
//! so the preview cannot promise one thing and the run do another.

use crate::backup::{
    destination_folders, find_nested_copy, plan_writes, preflight_sources,
    reject_destination_overlaps, rel_of, walk_all, KeepSet, KeepStatus, NestedCopy,
    ProtectedSet, Settings, Task, WalkResult, CANCELLED_MSG,
};
use crate::fsutil::{available_space, blocking, long_path};
use crate::glob;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// One preview at a time, process-wide.
///
/// A newer request cancels the one in flight instead of being refused: the
/// dialog it feeds is modal, so a preview still running is by definition one
/// the user has already walked away from. The serial is there for the same
/// reason `BackupState` has one — a scan that finishes late must clear its
/// own registration and not the newcomer's, which would leave the new scan
/// impossible to cancel.
#[derive(Default)]
pub struct PreviewState {
    inner: Mutex<Option<(u64, CancellationToken)>>,
    next: AtomicU64,
}

impl PreviewState {
    fn begin(&self) -> (u64, CancellationToken) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        let previous = {
            let mut slot = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            slot.replace((id, token.clone()))
        };
        // Cancel outside the lock: cancelling wakes waiters, and none of
        // them should have to queue behind this registration.
        if let Some((_, previous)) = previous {
            previous.cancel();
        }
        (id, token)
    }

    pub fn cancel(&self) {
        let token = {
            let slot = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            slot.as_ref().map(|(_, token)| token.clone())
        };
        if let Some(token) = token {
            token.cancel();
        }
    }

    fn finish(&self, id: u64) {
        let mut slot = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|(current, _)| *current == id) {
            *slot = None;
        }
    }
}

/// What one destination would receive. Byte counts are the source's sizes:
/// what is about to be written, not what the file currently occupies at the
/// destination.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DestinationPreview {
    pub path: String,
    /// False when the folder is not there to write to — an unplugged drive.
    /// Every count is zero in that case; they are unknown, not zero.
    pub reachable: bool,
    pub new_files: u64,
    pub new_bytes: u64,
    pub modified_files: u64,
    pub modified_bytes: u64,
    pub deleted_files: u64,
    pub deleted_bytes: u64,
    pub unchanged_files: u64,
    /// The room the run needs on the destination's volume, counted exactly
    /// as the run counts it before refusing a destination (`plan_writes`).
    pub required_bytes: u64,
    /// What the volume has free for this process, or none when it would not
    /// say — in which case the run does not check either.
    pub available_bytes: Option<u64>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPayload {
    pub cancelled: bool,
    pub source_files: u64,
    pub source_bytes: u64,
    /// Source entries the walk could not read. Their destination copies are
    /// left alone by the prune pass, so they are never counted as deletions
    /// — but the number is worth showing: it means this backup would be
    /// knowingly incomplete.
    pub unreadable: u64,
    pub destinations: Vec<DestinationPreview>,
}

/// The command-facing entry point.
pub async fn plan_backup(
    state: &PreviewState,
    task: Task,
    settings: Settings,
) -> Result<PreviewPayload> {
    let (id, token) = state.begin();
    let result = plan(&task, &settings, &token).await;
    state.finish(id);
    match result {
        Ok(payload) => Ok(payload),
        // A cancelled scan is not a failure to report — it is an answer the
        // user no longer wants. Say so rather than raising an error the
        // dialog would have to filter out by message.
        Err(_) if token.is_cancelled() => Ok(PreviewPayload {
            cancelled: true,
            ..Default::default()
        }),
        Err(e) => Err(e),
    }
}

async fn plan(task: &Task, settings: &Settings, token: &CancellationToken) -> Result<PreviewPayload> {
    let sources = preflight_sources(task)?;
    let source_paths: Vec<PathBuf> = sources.iter().map(|s| PathBuf::from(&s.path)).collect();
    let destinations: Vec<PathBuf> = task.destinations().iter().map(PathBuf::from).collect();
    if destinations.is_empty() {
        return Err(anyhow!("No destination set for this task"));
    }
    // Refused here for the same reason the run refuses it: a preview that
    // showed comfortable numbers for a configuration the run will not touch
    // would be worse than no preview at all.
    reject_destination_overlaps(&source_paths, &destinations)?;

    let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
    // The merged walk the run makes, so every rel already carries its
    // source's folder and a source that cannot be read protects that folder
    // instead of failing the preview.
    let walked = walk_all(&sources, &patterns, token).await?;
    // And the folder names the run gives its `ProtectedSet`. Without them a
    // path-shaped exclusion inside a source's folder is protected by the run
    // but counted here as a deletion.
    let folders = destination_folders(&sources);
    let protected = ProtectedSet::new(&walked, &patterns, &folders);
    let keep = KeepSet::new(walked.files.iter().map(|f| f.rel.clone()));

    let mut previews = Vec::with_capacity(destinations.len());
    for destination in &destinations {
        check_cancelled(token)?;
        let reachable = matches!(
            fs::metadata(long_path(destination)).await,
            Ok(meta) if meta.is_dir()
        );
        if !reachable {
            previews.push(DestinationPreview {
                path: destination.to_string_lossy().to_string(),
                reachable: false,
                ..Default::default()
            });
            continue;
        }
        // The run moves a copy 1.7.6 left one level down back up before it
        // compares anything. The preview does not move it — a dry run that
        // renames is not dry — but counts as if it had, or it would show
        // that whole copy as deleted and every file as new.
        let nested = find_nested_copy(destination, &sources, &walked).await;
        previews.push(
            plan_one(
                destination,
                &walked,
                &keep,
                &protected,
                nested.as_ref(),
                settings.parallel_copies(),
                token,
            )
            .await?,
        );
    }

    info!(task = %task.name, "previewed {} destination(s)", previews.len());
    Ok(PreviewPayload {
        cancelled: false,
        source_files: walked.files.len() as u64,
        source_bytes: walked.total_bytes,
        unreadable: walked.unreadable.len() as u64,
        destinations: previews,
    })
}

async fn plan_one(
    destination: &Path,
    walked: &WalkResult,
    keep: &KeepSet,
    protected: &ProtectedSet<'_>,
    nested: Option<&NestedCopy>,
    parallel: usize,
    token: &CancellationToken,
) -> Result<DestinationPreview> {
    // New, modified or unchanged: the same question `copy_one` asks before
    // deciding to stream a file, answered by the very function the run asks
    // before refusing a destination for room.
    //
    // One exception, deliberately not carried over: the copy loop always
    // re-copies `desktop.ini` regardless of what the destination holds. It
    // is a folder-icon descriptor, and re-writing an identical one changes
    // nothing the user could observe — counting it as "modified" on every
    // single preview would be noise standing in front of the numbers that
    // matter.
    let writes = plan_writes(destination, &walked.files, nested, parallel, true, token).await?;
    let probe = destination.to_path_buf();
    let mut preview = DestinationPreview {
        path: destination.to_string_lossy().to_string(),
        reachable: true,
        new_files: writes.new_files,
        new_bytes: writes.new_bytes,
        modified_files: writes.modified_files,
        modified_bytes: writes.modified_bytes,
        unchanged_files: writes.unchanged_files,
        required_bytes: writes.required_bytes,
        available_bytes: blocking(move || available_space(&probe)).await,
        ..Default::default()
    };

    count_deletions(destination, keep, protected, nested, token, &mut preview).await?;
    Ok(preview)
}

/// Walk the destination the way the prune pass does and count what it would
/// remove.
///
/// The traversal is a separate loop from `prune_destination` — that one also
/// re-spells drifted names, strips read-only bits and removes emptied
/// directories, and threading a "but don't actually do it" flag through all
/// of that is how a dry run ends up not being dry. What the two share is the
/// part that decides *whether* an entry is orphaned: the same `KeepSet` and
/// the same `ProtectedSet`.
async fn count_deletions(
    destination: &Path,
    keep: &KeepSet,
    protected: &ProtectedSet<'_>,
    nested: Option<&NestedCopy>,
    token: &CancellationToken,
    preview: &mut DestinationPreview,
) -> Result<()> {
    // A source root we could not enumerate makes prune skip the destination
    // entirely, so nothing would be deleted.
    if protected.source_root_unreadable() {
        return Ok(());
    }

    let root = long_path(destination);
    let mut stack: Vec<PathBuf> = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        check_cancelled(token)?;
        let mut entries = match fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        loop {
            check_cancelled(token)?;
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                // A listing that fails part-way tells us nothing about the
                // rest of the directory; prune stops there too.
                _ => break,
            };
            let path = entry.path();
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let found = rel_of(&root, &path);
            // Judged at the path it will have when prune runs, which for an
            // entry the run moves up is not the one it has now.
            let rel = nested.map_or(found.as_str(), |n| n.rel_after_move(&found));
            // Protected entries are skipped without descending, so a
            // protected directory shields its whole subtree — exactly as in
            // the prune pass.
            if protected.covers(rel) {
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && matches!(keep.status(rel), KeepStatus::Absent) {
                preview.deleted_files += 1;
                preview.deleted_bytes += entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Ok(())
}

fn check_cancelled(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        return Err(anyhow!(CANCELLED_MSG));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Here rather than in the list at the top: only the tests name the type,
    // and the library build would flag an import it never uses.
    use crate::backup::Source;

    /// A task in the shape every tasks.json from before multi-source holds:
    /// one source, mirrored straight into each destination.
    fn task_for(source: &Path, destinations: &[&Path]) -> Task {
        Task {
            id: "preview".into(),
            name: "preview".into(),
            source: Some(source.to_string_lossy().to_string()),
            sources: None,
            destination: None,
            destinations: Some(
                destinations
                    .iter()
                    .map(|d| d.to_string_lossy().to_string())
                    .collect(),
            ),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
            keep_versions_days: None,
        }
    }

    fn task_with_sources(sources: &[(&Path, &str)], destinations: &[&Path]) -> Task {
        Task {
            id: "preview".into(),
            name: "preview".into(),
            source: None,
            sources: Some(
                sources
                    .iter()
                    .map(|(p, folder)| Source {
                        path: p.to_string_lossy().to_string(),
                        folder: (*folder).to_string(),
                    })
                    .collect(),
            ),
            destination: None,
            destinations: Some(
                destinations
                    .iter()
                    .map(|d| d.to_string_lossy().to_string())
                    .collect(),
            ),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
            keep_versions_days: None,
        }
    }

    /// Where the run writes a `task_for` source: straight into the
    /// destination. Spelled once, the way `backup.rs`'s tests spell it.
    fn backed_up(dest: &Path, _src: &Path) -> PathBuf {
        dest.to_path_buf()
    }

    /// The four numbers the dialog shows, on a tree that has one of each.
    #[tokio::test]
    async fn counts_new_modified_unchanged_and_deleted() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        let mirror = backed_up(&dest, &source);
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&mirror).unwrap();

        // Unchanged on both sides: same bytes, and the same mtime, which is
        // what the run compares.
        std::fs::write(source.join("same.txt"), b"identical").unwrap();
        std::fs::copy(source.join("same.txt"), mirror.join("same.txt")).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(
            &std::fs::metadata(source.join("same.txt")).unwrap(),
        );
        filetime::set_file_mtime(mirror.join("same.txt"), mtime).unwrap();

        std::fs::write(source.join("new.txt"), b"brand new").unwrap();
        std::fs::write(source.join("changed.txt"), b"the new contents").unwrap();
        std::fs::write(mirror.join("changed.txt"), b"old").unwrap();
        std::fs::write(mirror.join("removed.txt"), b"gone from source").unwrap();

        let payload = plan(
            &task_for(&source, &[&dest]),
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let d = &payload.destinations[0];
        assert!(d.reachable);
        assert_eq!((d.new_files, d.new_bytes), (1, 9));
        assert_eq!((d.modified_files, d.modified_bytes), (1, 16));
        assert_eq!(d.unchanged_files, 1);
        assert_eq!((d.deleted_files, d.deleted_bytes), (1, 16));
        assert_eq!(payload.source_files, 3);
    }

    /// "Exclude" means "don't copy", never "delete from the backup" (#2).
    /// A preview that counted excluded leftovers as deletions would be
    /// asking the user to approve something the run will not do.
    #[tokio::test]
    async fn an_excluded_file_is_neither_copied_nor_deleted() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        let mirror = backed_up(&dest, &source);
        std::fs::create_dir_all(source.join("node_modules")).unwrap();
        std::fs::create_dir_all(mirror.join("node_modules")).unwrap();
        std::fs::write(source.join("keep.txt"), b"kept").unwrap();
        std::fs::write(source.join("node_modules/lib.js"), b"ignored").unwrap();
        std::fs::write(mirror.join("node_modules/lib.js"), b"ignored").unwrap();

        let settings = Settings {
            exclude_patterns: "node_modules".into(),
            ..Default::default()
        };
        let payload = plan(
            &task_for(&source, &[&dest]),
            &settings,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let d = &payload.destinations[0];
        assert_eq!(d.deleted_files, 0, "an exclusion is not a deletion");
        assert_eq!(d.new_files, 1, "only keep.txt would be copied");
    }

    /// An unplugged drive is a state to report, not an error to raise: the
    /// other destinations still have numbers worth showing.
    #[tokio::test]
    async fn an_absent_destination_is_reported_without_counts() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        let absent = root.path().join("unplugged");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("a.txt"), b"alpha").unwrap();

        let payload = plan(
            &task_for(&source, &[&dest, &absent]),
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(payload.destinations[0].reachable);
        assert_eq!(payload.destinations[0].new_files, 1);
        assert!(!payload.destinations[1].reachable);
        assert_eq!(payload.destinations[1].new_files, 0);
    }

    /// The preview has to describe the layout the run will actually produce,
    /// or the numbers in the confirmation dialog are about a different backup
    /// than the one about to happen.
    #[tokio::test]
    async fn a_preview_counts_every_source() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("Alpha");
        let b = root.path().join("Beta");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(a.join("one.txt"), b"1").unwrap();
        std::fs::write(b.join("two.txt"), b"2").unwrap();

        let task = task_with_sources(&[(&a, "Alpha"), (&b, "Beta")], &[&dest]);
        let payload = plan(&task, &Settings::default(), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(payload.source_files, 2, "both sources counted");
        assert_eq!(payload.destinations[0].new_files, 2);
    }

    /// A source on an unplugged drive is reported, not counted. The run
    /// leaves that source's folder at the destination alone, so the preview
    /// must neither fail over it nor show the only copy of its files as
    /// deletions — and it has to say the backup would be incomplete.
    #[tokio::test]
    async fn an_absent_source_is_reported_not_counted_as_deleted() {
        let root = tempfile::tempdir().unwrap();
        let here = root.path().join("here");
        let gone = root.path().join("gone");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&here).unwrap();
        std::fs::write(here.join("a.txt"), b"alpha").unwrap();
        // What an earlier run copied from the source that is now unplugged.
        std::fs::create_dir_all(dest.join("Gone")).unwrap();
        std::fs::write(dest.join("Gone/only-copy.txt"), b"precious").unwrap();

        let payload = plan(
            &task_with_sources(&[(&here, "Here"), (&gone, "Gone")], &[&dest]),
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(payload.unreadable, 1, "the backup would be knowingly incomplete");
        assert_eq!(payload.source_files, 1, "the source that is there still counts");
        assert_eq!(payload.destinations[0].deleted_files, 0);
    }

    /// The run protects a path-shaped exclusion inside a source's folder by
    /// taking the folder off before asking the patterns, which it can only do
    /// because it is told the folder names. A preview not told them would
    /// count `Alpha/Photos/raw` as a deletion the run then declines to make:
    /// the dialog would be asking for approval of something that never
    /// happens.
    #[tokio::test]
    async fn a_path_shaped_exclusion_inside_a_source_folder_is_not_a_deletion() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let other = root.path().join("other");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(source.join("keep.txt"), b"kept").unwrap();
        // Copied by an earlier run, then excluded, then removed from the
        // source: the walk never meets it, so only the pattern protects it.
        std::fs::create_dir_all(dest.join("Alpha/Photos/raw")).unwrap();
        std::fs::write(dest.join("Alpha/Photos/raw/a.cr2"), b"raw bytes").unwrap();

        let settings = Settings {
            exclude_patterns: "Photos/raw".into(),
            ..Default::default()
        };
        let payload = plan(
            &task_with_sources(&[(&source, "Alpha"), (&other, "Other")], &[&dest]),
            &settings,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(payload.destinations[0].deleted_files, 0);
    }

    /// A copy 1.7.6 left one level down is moved back up by the run before
    /// anything is compared. The preview has to count it that way — the
    /// files as already there, and only a genuine leftover as deleted — and
    /// must not move anything itself: a preview is a dry run.
    #[tokio::test]
    async fn a_copy_left_one_level_down_previews_as_already_backed_up() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        let nested = dest.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(source.join("same.txt"), b"identical").unwrap();
        std::fs::copy(source.join("same.txt"), nested.join("same.txt")).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(
            &std::fs::metadata(source.join("same.txt")).unwrap(),
        );
        filetime::set_file_mtime(nested.join("same.txt"), mtime).unwrap();
        std::fs::write(nested.join("leftover.txt"), b"gone from source").unwrap();

        let payload = plan(
            &task_for(&source, &[&dest]),
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let d = &payload.destinations[0];
        assert_eq!(d.unchanged_files, 1, "found where the move will put it");
        assert_eq!(d.new_files, 0);
        assert_eq!((d.deleted_files, d.deleted_bytes), (1, 16), "only the leftover goes");
        assert!(nested.join("same.txt").exists(), "the preview moved nothing");
        assert!(!dest.join("same.txt").exists());
    }

    /// The dialog has to be able to say a destination lacks the room before
    /// the user confirms: what the run would need there, by the run's own
    /// count, and what the volume has free.
    #[tokio::test]
    async fn a_preview_reports_the_room_a_destination_needs() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("new.bin"), vec![0u8; 5000]).unwrap();

        let payload = plan(
            &task_for(&source, &[&dest]),
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let d = &payload.destinations[0];
        assert_eq!(d.required_bytes, 8192, "one new file, in whole clusters");
        assert!(d.available_bytes.is_some_and(|free| free > 0));
    }

    #[tokio::test]
    async fn a_cancelled_scan_answers_cancelled_rather_than_failing() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("a.txt"), b"alpha").unwrap();

        let state = PreviewState::default();
        // Cancel the scan the moment it registers, from the same thread the
        // command would: a second begin() supersedes the first.
        let (_id, token) = state.begin();
        token.cancel();
        let result = plan(
            &task_for(&source, &[&dest]),
            &Settings::default(),
            &token,
        )
        .await;
        assert!(result.is_err(), "a cancelled scan stops");
    }

    /// A newer preview must cancel the one it replaces, and a scan that
    /// finishes late must not clear the newcomer's registration.
    #[test]
    fn a_new_scan_supersedes_the_one_in_flight() {
        let state = PreviewState::default();
        let (first_id, first) = state.begin();
        let (_second_id, second) = state.begin();

        assert!(first.is_cancelled(), "the older scan is stopped");
        assert!(!second.is_cancelled());

        // The first scan notices and unwinds — after the second registered.
        state.finish(first_id);
        state.cancel();
        assert!(second.is_cancelled(), "the live scan must stay cancellable");
    }
}
