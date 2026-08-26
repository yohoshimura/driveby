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
    preflight_source, reject_destination_overlaps, rel_of, same_mtime, walk, KeepSet, KeepStatus,
    ProtectedSet, Settings, Task, WalkResult, CANCELLED_MSG,
};
use crate::fsutil::long_path;
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
    let source = preflight_source(task).await?;
    let destinations: Vec<PathBuf> = task.destinations().iter().map(PathBuf::from).collect();
    if destinations.is_empty() {
        return Err(anyhow!("No destination set for this task"));
    }
    // Refused here for the same reason the run refuses it: a preview that
    // showed comfortable numbers for a configuration the run will not touch
    // would be worse than no preview at all.
    reject_destination_overlaps(&source, &destinations)?;

    let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
    let walked = walk(&source, &patterns).await?;
    let protected = ProtectedSet::new(&walked, &patterns);
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
        previews.push(plan_one(destination, &walked, &keep, &protected, token).await?);
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
    token: &CancellationToken,
) -> Result<DestinationPreview> {
    let mut preview = DestinationPreview {
        path: destination.to_string_lossy().to_string(),
        reachable: true,
        ..Default::default()
    };

    // New, modified or unchanged: the same question `copy_one` asks before
    // deciding to stream a file, answered with the same size-and-mtime
    // comparison.
    //
    // One exception, deliberately not carried over: the copy loop always
    // re-copies `desktop.ini` regardless of what the destination holds. It
    // is a folder-icon descriptor, and re-writing an identical one changes
    // nothing the user could observe — counting it as "modified" on every
    // single preview would be noise standing in front of the numbers that
    // matter.
    for (i, file) in walked.files.iter().enumerate() {
        // The cost here is one stat per file; checking the token every few
        // hundred keeps a Cancel responsive without making the check itself
        // the expensive part.
        if i % 256 == 0 {
            check_cancelled(token)?;
        }
        match fs::metadata(long_path(&destination.join(&file.rel))).await {
            Err(_) => {
                preview.new_files += 1;
                preview.new_bytes += file.size;
            }
            Ok(meta)
                if meta.is_file()
                    && meta.len() == file.size
                    && meta
                        .modified()
                        .ok()
                        .is_some_and(|m| same_mtime(m, file.mtime)) =>
            {
                preview.unchanged_files += 1
            }
            Ok(_) => {
                preview.modified_files += 1;
                preview.modified_bytes += file.size;
            }
        }
    }

    count_deletions(destination, keep, protected, token, &mut preview).await?;
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
            let rel = rel_of(&root, &path);
            // Protected entries are skipped without descending, so a
            // protected directory shields its whole subtree — exactly as in
            // the prune pass.
            if protected.covers(&rel) {
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && matches!(keep.status(&rel), KeepStatus::Absent) {
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

    fn task_for(source: &Path, destinations: &[&Path]) -> Task {
        Task {
            id: "preview".into(),
            name: "preview".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(
                destinations
                    .iter()
                    .map(|d| d.to_string_lossy().to_string())
                    .collect(),
            ),
            schedule: None,
            last_backup: None,
        }
    }

    /// The four numbers the dialog shows, on a tree that has one of each.
    #[tokio::test]
    async fn counts_new_modified_unchanged_and_deleted() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Unchanged on both sides: same bytes, and the same mtime, which is
        // what the run compares.
        std::fs::write(source.join("same.txt"), b"identical").unwrap();
        std::fs::copy(source.join("same.txt"), dest.join("same.txt")).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(
            &std::fs::metadata(source.join("same.txt")).unwrap(),
        );
        filetime::set_file_mtime(dest.join("same.txt"), mtime).unwrap();

        std::fs::write(source.join("new.txt"), b"brand new").unwrap();
        std::fs::write(source.join("changed.txt"), b"the new contents").unwrap();
        std::fs::write(dest.join("changed.txt"), b"old").unwrap();
        std::fs::write(dest.join("removed.txt"), b"gone from source").unwrap();

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
        std::fs::create_dir_all(source.join("node_modules")).unwrap();
        std::fs::create_dir_all(dest.join("node_modules")).unwrap();
        std::fs::write(source.join("keep.txt"), b"kept").unwrap();
        std::fs::write(source.join("node_modules/lib.js"), b"ignored").unwrap();
        std::fs::write(dest.join("node_modules/lib.js"), b"ignored").unwrap();

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
