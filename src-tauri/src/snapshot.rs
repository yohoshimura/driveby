//! Daily versions: a destination that keeps one whole snapshot per day, every
//! file unchanged since the day before hard-linked to that day's copy.
//!
//! ```text
//! <dest>/.driveby-snapshots      the marker: its presence makes this layout
//! <dest>/2026-09-21/             a committed snapshot, the whole tree
//! <dest>/.driveby-in-progress/   the snapshot being built; resumable
//! ```
//!
//! The design, and why each step is ordered the way it is:
//! docs/superpowers/specs/2026-09-23-daily-versions-design.md.

use crate::backup::{entry_names, is_absent, CANCELLED_MSG};
use crate::fsutil::{
    apply_attrs, blocking, clear_readonly, long_path, read_attrs, remove_link_safe,
};
use crate::persist;
use anyhow::{anyhow, Context, Result};
use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub(crate) const MARKER: &str = ".driveby-snapshots";
pub(crate) const IN_PROGRESS: &str = ".driveby-in-progress";

/// Names Driveby owns at a destination root. A source must not bring either
/// there: another destination's marker, mirrored onto this root, would make
/// it read as daily versions.
pub(crate) fn is_reserved_name(name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case(MARKER) || name.eq_ignore_ascii_case(IN_PROGRESS)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct Marker {
    pub(crate) version: u32,
    /// Set only while versions are being turned off: the snapshot that is
    /// becoming the mirror again.
    #[serde(default)]
    pub(crate) leaving: Option<String>,
    /// Set once every other snapshot is gone, before anything moves up. From
    /// then on a folder at the root named like a date may be a source folder
    /// that already came up, and must not be deleted as a snapshot.
    #[serde(default)]
    pub(crate) cleared: bool,
}

impl Default for Marker {
    fn default() -> Self {
        Self { version: 1, leaving: None, cleared: false }
    }
}

/// The marker at `destination`, None when there is none.
///
/// A marker that is there but cannot be parsed still marks the layout — its
/// presence is the fact that matters — and reads as the default. One that
/// cannot be *read* is an error: answering "no marker" would hand the
/// destination to a mirror prune, which deletes every snapshot as an orphan.
pub(crate) async fn read_marker(destination: &Path) -> Result<Option<Marker>> {
    let path = long_path(&destination.join(MARKER));
    match fs::read(&path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).unwrap_or_default())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

pub(crate) async fn write_marker(destination: &Path, marker: &Marker) -> Result<()> {
    persist::write_json_atomic(&long_path(&destination.join(MARKER)), marker).await
}

/// Remove the marker; nothing to do when it is already gone.
pub(crate) async fn remove_marker(destination: &Path) -> Result<()> {
    match fs::remove_file(long_path(&destination.join(MARKER))).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// A committed snapshot: a folder at the destination root named for its day.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) date: NaiveDate,
    pub(crate) path: PathBuf,
}

impl Snapshot {
    pub(crate) fn name(&self) -> String {
        day_name(self.date)
    }
}

pub(crate) fn day_name(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// A snapshot folder's date, or None for any other name. Strict: exactly
/// `YYYY-MM-DD`, so `2026-9-3` or `2026-09-23 (copy)` is not a snapshot.
pub(crate) fn parse_day(name: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(name, "%Y-%m-%d").ok()?;
    // chrono accepts `2026-9-3` and a signed year; the round trip does not.
    (day_name(date) == name).then_some(date)
}

/// The committed snapshots at `destination`, oldest first.
pub(crate) async fn list(destination: &Path) -> Result<Vec<Snapshot>> {
    let root = long_path(destination);
    let mut entries = fs::read_dir(&root)
        .await
        .with_context(|| format!("list {}", root.display()))?;
    let mut snapshots = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        // `file_type` does not follow links: a symlink named like a date is
        // not a snapshot.
        if !entry.file_type().await.is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(date) = parse_day(&name) {
            snapshots.push(Snapshot { date, path: destination.join(&name) });
        }
    }
    snapshots.sort_by_key(|s| s.date);
    Ok(snapshots)
}

pub(crate) async fn is_dir(path: &Path) -> bool {
    fs::metadata(long_path(path)).await.is_ok_and(|m| m.is_dir())
}

/// Today's local date — or, in a debug build, `DRIVEBY_TODAY` (YYYY-MM-DD),
/// so a run on another day can be tried by hand in `npm run tauri dev`.
pub(crate) fn clock_today() -> NaiveDate {
    #[cfg(debug_assertions)]
    if let Some(date) =
        std::env::var("DRIVEBY_TODAY").ok().as_deref().and_then(parse_day)
    {
        return date;
    }
    chrono::Local::now().date_naive()
}

/// The day a run writes: the clock's, or the newest snapshot's when the clock
/// reads earlier. An older-named folder holding the latest state would be
/// taken for history, and retention would delete it first.
pub(crate) fn effective_day(clock: NaiveDate, snapshots: &[Snapshot]) -> NaiveDate {
    snapshots.last().map_or(clock, |newest| clock.max(newest.date))
}

/// What retention deletes: the snapshots dated before `day − keep_days`, but
/// never the newest. A task that has not run for months keeps its last one
/// rather than starting over from nothing.
pub(crate) fn expired(snapshots: &[Snapshot], day: NaiveDate, keep_days: u32) -> Vec<Snapshot> {
    let Some(cutoff) = day.checked_sub_days(Days::new(u64::from(keep_days))) else {
        return Vec::new();
    };
    let Some((_newest, older)) = snapshots.split_last() else {
        return Vec::new();
    };
    older.iter().filter(|s| s.date < cutoff).cloned().collect()
}

/// What may go, oldest first, when a destination lacks room: everything but
/// the newest snapshot, which today's is built from or is.
pub(crate) fn evictable(snapshots: &[Snapshot]) -> Vec<Snapshot> {
    snapshots.split_last().map(|(_, older)| older.to_vec()).unwrap_or_default()
}

/// Delete a tree Driveby owns — an expired snapshot, a stale
/// `.driveby-in-progress` — without touching the attributes of the files it
/// shares with other snapshots.
///
/// Files go through `remove_link_safe`: prune's clear-then-delete would strip
/// `+R` from every snapshot sharing the file. A symlink is removed as a link
/// and never followed. Directories are never shared, so their own `+R` (a
/// custom folder icon) is cleared before they go, deepest first.
///
/// One hop onto the blocking pool for the whole tree: a snapshot is as large
/// as the backup, and a hop per file would cost more than the work.
pub(crate) async fn remove_tree(root: &Path, token: &CancellationToken) -> Result<()> {
    let (root, token) = (root.to_path_buf(), token.clone());
    blocking(move || remove_tree_sync(&root, &token)).await
}

fn remove_tree_sync(root: &Path, token: &CancellationToken) -> Result<()> {
    let root = long_path(root);
    let mut dirs = vec![root.clone()];
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let listing = std::fs::read_dir(&dir).with_context(|| format!("list {}", dir.display()))?;
        for entry in listing {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                // A link to a directory is removed with remove_dir on Windows.
                if std::fs::remove_file(&path).is_err() {
                    std::fs::remove_dir(&path)
                        .with_context(|| format!("remove {}", path.display()))?;
                }
            } else if kind.is_dir() {
                dirs.push(path.clone());
                stack.push(path);
            } else {
                remove_link_safe(&path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.as_os_str().len()));
    for dir in dirs {
        clear_readonly(&dir);
        std::fs::remove_dir(&dir).with_context(|| format!("remove {}", dir.display()))?;
    }
    Ok(())
}

/// What `clone_tree` did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct CloneStats {
    pub(crate) linked: u64,
    pub(crate) copied: u64,
}

/// Fill `into` with hard links to every file of `from` — rsnapshot's
/// `cp -al` — so today's snapshot starts as yesterday's at the cost of one
/// directory entry per file. The sync that follows only ever *replaces* an
/// entry (scratch file, then rename) or removes one, so nothing it does in
/// `into` reaches `from`.
///
/// Resumable: an entry already in `into` is left as it is, whether it comes
/// from an interrupted clone or from a sync that had already started.
/// Symlinks and scratch files (`*.driveby-tmp`) are left out. A file that
/// cannot be linked (NTFS allows 1023 links to one file) is copied instead.
/// Directories get `from`'s attribute bits, deepest first, so a custom folder
/// icon survives even where the source no longer lists the folder.
pub(crate) async fn clone_tree(
    from: &Path,
    into: &Path,
    token: &CancellationToken,
) -> Result<CloneStats> {
    let (from, into, token) = (from.to_path_buf(), into.to_path_buf(), token.clone());
    blocking(move || clone_tree_sync(&from, &into, &token)).await
}

fn clone_tree_sync(
    from: &Path,
    into: &Path,
    token: &CancellationToken,
) -> Result<CloneStats> {
    let mut stats = CloneStats::default();
    let from_root = long_path(from);
    let into_root = long_path(into);
    std::fs::create_dir_all(&into_root)
        .with_context(|| format!("create {}", into_root.display()))?;
    let mut dirs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut stack = vec![from_root.clone()];
    while let Some(dir) = stack.pop() {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let listing = std::fs::read_dir(&dir)
            .with_context(|| format!("list {}", dir.display()))?;
        for entry in listing {
            let entry = entry?;
            let src = entry.path();
            let dst =
                into_root.join(src.strip_prefix(&from_root).unwrap_or(&src));
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                std::fs::create_dir_all(&dst)
                    .with_context(|| format!("create {}", dst.display()))?;
                dirs.push((src.clone(), dst));
                stack.push(src);
            } else if kind.is_file() {
                if entry.file_name().to_string_lossy().ends_with(".driveby-tmp") {
                    continue;
                }
                if std::fs::symlink_metadata(&dst).is_ok() {
                    continue;
                }
                match std::fs::hard_link(&src, &dst) {
                    Ok(()) => stats.linked += 1,
                    Err(e) => {
                        warn!("could not link {} ({}); copying it", dst.display(), e);
                        std::fs::copy(&src, &dst)
                            .with_context(|| format!("copy {}", dst.display()))?;
                        stats.copied += 1;
                    }
                }
            }
        }
    }
    // After the files, deepest first: a folder's bits go on once what it
    // holds is in place, the order `restore_dirs` uses.
    dirs.sort_by_key(|(_, d)| std::cmp::Reverse(d.as_os_str().len()));
    for (src, dst) in dirs {
        if let Some(attrs) = read_attrs(&src) {
            apply_attrs(&dst, attrs);
        }
    }
    Ok(stats)
}

/// What `move_entries` did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Moved {
    pub(crate) moved: u64,
    /// Entries left where they were, because their name was taken in `into`.
    pub(crate) blocked: Vec<String>,
}

/// Rename every entry of `from` into `into`, except the names in `skip`
/// (compared without case). Renames only, on one volume: instant, no room
/// needed, nothing deleted.
///
/// A name already taken in `into` is left where it is and reported. That is
/// asked again right before each rename, because `fs::rename` replaces an
/// existing file on Windows, and what it would replace is a copy this step
/// promises to keep. A rename that fails is an error naming the entry; the
/// caller stops there, and the next run finds the rest where it was.
pub(crate) async fn move_entries(
    from: &Path,
    into: &Path,
    skip: &[&str],
) -> Result<Moved> {
    let mut report = Moved::default();
    // Listed before anything moves: renaming out of a directory while its
    // listing is still being read can skip entries.
    for name in entry_names(from).await {
        if skip.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let to = into.join(&name);
        if !is_absent(&to).await {
            report.blocked.push(name);
            continue;
        }
        fs::rename(long_path(&from.join(&name)), long_path(&to))
            .await
            .with_context(|| format!("move {} into {}", name, into.display()))?;
        report.moved += 1;
    }
    Ok(report)
}

/// Where one destination's run writes, decided before its copy phase.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Plan {
    /// A plain mirror at the destination root. `versions_unavailable` when
    /// the task keeps versions and this drive cannot make hard links.
    Mirror { versions_unavailable: bool },
    /// A snapshot. `target` is the day's own folder, updated in place, or
    /// `.driveby-in-progress`, which `commit` renames to `day`.
    Snapshot { target: PathBuf, day: String, in_progress: bool },
}

impl Plan {
    /// The folder the pipeline writes into.
    pub(crate) fn target<'a>(&'a self, destination: &'a Path) -> &'a Path {
        match self {
            Plan::Mirror { .. } => destination,
            Plan::Snapshot { target, .. } => target,
        }
    }
}

/// Get `destination` ready for a run that keeps versions: move an existing
/// mirror into the first snapshot, apply retention, and pick and fill the
/// target. `can_link` is `fsutil::hard_link_supported`'s answer for this
/// destination, taken by the caller.
pub(crate) async fn prepare(
    destination: &Path,
    keep_days: u32,
    clock: NaiveDate,
    can_link: bool,
    token: &CancellationToken,
) -> Result<Plan> {
    let mut marker = read_marker(destination).await?;
    // Turned off, then on again before turning off had finished: the user
    // confirmed deleting those versions, so that is finished first.
    if marker.as_ref().is_some_and(|m| m.leaving.is_some()) {
        leave(destination, token).await?;
        marker = None;
    }
    if !can_link {
        if marker.is_some() {
            return Err(anyhow!(
                "This destination keeps daily versions, but hard links cannot be made there \
                 now. Nothing was changed."
            ));
        }
        // Finishes an interrupted first move, if there is one, so the mirror
        // this run writes to is whole.
        leave(destination, token).await?;
        return Ok(Plan::Mirror { versions_unavailable: true });
    }

    let in_progress = destination.join(IN_PROGRESS);
    if marker.is_none() {
        // The first run with versions: the mirror at the root becomes the
        // first snapshot, by renaming. The marker goes last, so a move that
        // stops half-way is simply carried on by the next run.
        fs::create_dir_all(long_path(&in_progress)).await?;
        let moved = move_entries(destination, &in_progress, &[IN_PROGRESS, "desktop.ini"]).await?;
        for name in &moved.blocked {
            warn!(
                dest = %destination.display(),
                "{} is already in the first version; left at the root", name
            );
        }
        write_marker(destination, &Marker::default()).await?;
        info!(
            dest = %destination.display(),
            "keeping daily versions from now on; {} entries moved into the first one", moved.moved
        );
    }

    let snapshots = list(destination).await?;
    let day = effective_day(clock, &snapshots);
    let name = day_name(day);
    for old in expired(&snapshots, day, keep_days) {
        info!(dest = %destination.display(), "deleting the version of {}", old.name());
        remove_tree(&old.path, token).await?;
    }
    let today = destination.join(&name);
    if is_dir(&today).await {
        // A later run on the same day updates that day in place, and a
        // `.driveby-in-progress` beside it can only be a leftover.
        if is_dir(&in_progress).await {
            remove_tree(&in_progress, token).await?;
        }
        return Ok(Plan::Snapshot { target: today, day: name, in_progress: false });
    }
    fs::create_dir_all(long_path(&in_progress)).await?;
    // Retention never deletes the newest, so it is still there to clone.
    if let Some(newest) = snapshots.last() {
        let cloned = clone_tree(&newest.path, &in_progress, token).await?;
        info!(
            dest = %destination.display(),
            "started {} from {}: {} linked, {} copied",
            name,
            newest.name(),
            cloned.linked,
            cloned.copied
        );
    }
    Ok(Plan::Snapshot { target: in_progress, day: name, in_progress: true })
}

/// Turn a destination with daily versions back into a mirror: the newest
/// snapshot comes up to the root and the others are deleted. Also finishes a
/// first move into snapshots that was interrupted before its marker was
/// written. A mirror is left as it is.
///
/// Every step can be redone after an interruption. `leaving` is written
/// before anything is deleted and `cleared` before anything moves, so a run
/// that stops half-way knows which folder is coming up, and never takes a
/// source folder named like a date for a snapshot.
pub(crate) async fn leave(destination: &Path, token: &CancellationToken) -> Result<()> {
    let in_progress = destination.join(IN_PROGRESS);
    let Some(mut marker) = read_marker(destination).await? else {
        if is_dir(&in_progress).await {
            come_up(destination, &in_progress).await?;
        }
        return Ok(());
    };
    let leaving = match marker.leaving.clone() {
        Some(name) => name,
        None => {
            let name = match list(destination).await?.last() {
                Some(newest) => newest.name(),
                None if is_dir(&in_progress).await => IN_PROGRESS.to_string(),
                None => return remove_marker(destination).await,
            };
            marker.leaving = Some(name.clone());
            write_marker(destination, &marker).await?;
            name
        }
    };
    if !marker.cleared {
        for snapshot in list(destination).await? {
            if snapshot.name() != leaving {
                remove_tree(&snapshot.path, token).await?;
            }
        }
        if leaving != IN_PROGRESS && is_dir(&in_progress).await {
            remove_tree(&in_progress, token).await?;
        }
        marker.cleared = true;
        write_marker(destination, &marker).await?;
    }
    let from = destination.join(&leaving);
    if is_dir(&from).await {
        come_up(destination, &from).await?;
    }
    info!(
        dest = %destination.display(),
        "daily versions turned off; {} is the backup again",
        leaving
    );
    remove_marker(destination).await
}

/// Move `from`'s entries up to `destination`, then remove the emptied folder.
async fn come_up(destination: &Path, from: &Path) -> Result<()> {
    let moved = move_entries(from, destination, &[]).await?;
    for name in &moved.blocked {
        // The destination root's own folder icon stays. A snapshot's
        // `desktop.ini` is the source's, which the mirror copies again anyway.
        if name.eq_ignore_ascii_case("desktop.ini") {
            let copy = from.join(name);
            blocking(move || remove_link_safe(&copy))
                .await
                .with_context(|| format!("remove {}", from.join(name).display()))?;
            continue;
        }
        return Err(anyhow!(
            "{} cannot come back to {}: an entry of that name is already there",
            name,
            destination.display()
        ));
    }
    let dir = from.to_path_buf();
    blocking(move || {
        // Emptied now. Its own ReadOnly bit (a custom folder icon) would block
        // the removal, and a directory is never shared with another snapshot.
        clear_readonly(&dir);
        std::fs::remove_dir(long_path(&dir))
    })
    .await
    .with_context(|| format!("remove {}", from.display()))
}

/// Make the snapshot a run wrote into its day: rename `.driveby-in-progress`
/// to the day's name. A day updated in place is already where it belongs.
pub(crate) async fn commit(destination: &Path, plan: &Plan) -> Result<()> {
    if let Plan::Snapshot { target, day, in_progress: true } = plan {
        fs::rename(long_path(target), long_path(&destination.join(day)))
            .await
            .with_context(|| format!("commit the version of {}", day))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn d(s: &str) -> NaiveDate {
        parse_day(s).unwrap()
    }

    fn snap(root: &Path, s: &str) -> Snapshot {
        Snapshot { date: d(s), path: root.join(s) }
    }

    fn go() -> CancellationToken {
        CancellationToken::new()
    }

    fn tree(root: &Path, files: &[(&str, &str)]) {
        for (rel, body) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    fn names_at(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    use crate::fsutil::same_file;

    async fn mark(dest: &Path) {
        write_marker(dest, &Marker::default()).await.unwrap();
    }

    fn in_progress_on(dest: &Path, day: &str) -> Plan {
        Plan::Snapshot { target: dest.join(IN_PROGRESS), day: day.into(), in_progress: true }
    }

    #[tokio::test]
    async fn the_first_run_turns_the_mirror_into_the_first_day_by_renaming() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("a.txt", "a"), ("sub/b.txt", "b"), ("desktop.ini", "[icon]")]);

        let plan = prepare(dest, 30, d("2026-09-21"), true, &go()).await.unwrap();

        assert_eq!(plan, in_progress_on(dest, "2026-09-21"));
        assert_eq!(names_at(dest), [IN_PROGRESS, MARKER, "desktop.ini"]);
        assert_eq!(std::fs::read(dest.join(IN_PROGRESS).join("sub/b.txt")).unwrap(), b"b");
        commit(dest, &plan).await.unwrap();
        assert_eq!(names_at(dest), [MARKER, "2026-09-21", "desktop.ini"]);
    }

    #[tokio::test]
    async fn the_next_day_starts_as_a_clone_of_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-21"), &[("a.txt", "a")]);
        mark(dest).await;

        let plan = prepare(dest, 30, d("2026-09-22"), true, &go()).await.unwrap();

        assert_eq!(plan, in_progress_on(dest, "2026-09-22"));
        assert!(same_file(&dest.join("2026-09-21/a.txt"), &dest.join(IN_PROGRESS).join("a.txt")));
    }

    #[tokio::test]
    async fn a_later_run_the_same_day_updates_that_day_and_drops_a_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        tree(&dest.join(IN_PROGRESS), &[("stale.txt", "s")]);
        mark(dest).await;

        let plan = prepare(dest, 30, d("2026-09-22"), true, &go()).await.unwrap();

        let today = dest.join("2026-09-22");
        assert_eq!(
            plan,
            Plan::Snapshot { target: today.clone(), day: "2026-09-22".into(), in_progress: false }
        );
        assert!(!dest.join(IN_PROGRESS).exists());
        commit(dest, &plan).await.unwrap();
        assert!(today.join("a.txt").exists(), "a day updated in place has nothing to rename");
    }

    #[tokio::test]
    async fn retention_deletes_old_days_but_never_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        for day in ["2026-08-01", "2026-09-20", "2026-09-22"] {
            tree(&dest.join(day), &[("a.txt", "a")]);
        }
        mark(dest).await;

        prepare(dest, 7, d("2026-09-23"), true, &go()).await.unwrap();

        assert!(!dest.join("2026-08-01").exists());
        assert!(dest.join("2026-09-20").exists());
        assert!(dest.join("2026-09-22").exists());

        let idle = tempfile::tempdir().unwrap();
        tree(&idle.path().join("2026-01-01"), &[("a.txt", "a")]);
        mark(idle.path()).await;
        prepare(idle.path(), 7, d("2026-09-23"), true, &go()).await.unwrap();
        assert!(idle.path().join("2026-01-01").exists(), "months without a run keep the last day");
    }

    #[tokio::test]
    async fn a_clock_behind_the_newest_day_writes_into_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-23"), &[("a.txt", "a")]);
        mark(dest).await;

        let plan = prepare(dest, 30, d("2026-09-20"), true, &go()).await.unwrap();

        assert_eq!(
            plan,
            Plan::Snapshot {
                target: dest.join("2026-09-23"),
                day: "2026-09-23".into(),
                in_progress: false,
            }
        );
        assert!(!dest.join("2026-09-20").exists());
        assert!(!dest.join(IN_PROGRESS).exists());
    }

    #[tokio::test]
    async fn without_hard_links_a_mirror_stays_a_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("a.txt", "a")]);

        let plan = prepare(dest, 30, d("2026-09-21"), false, &go()).await.unwrap();

        assert_eq!(plan, Plan::Mirror { versions_unavailable: true });
        assert_eq!(names_at(dest), ["a.txt"]);
    }

    #[tokio::test]
    async fn without_hard_links_a_destination_with_versions_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-21"), &[("a.txt", "a")]);
        mark(dest).await;

        let err = prepare(dest, 30, d("2026-09-22"), false, &go()).await.unwrap_err();

        assert!(err.to_string().contains("hard links"), "{err}");
        assert_eq!(names_at(dest), [MARKER, "2026-09-21"]);
    }

    #[tokio::test]
    async fn turning_off_brings_the_newest_day_up_and_deletes_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-21"), &[("old.txt", "old")]);
        tree(
            &dest.join("2026-09-22"),
            &[("a.txt", "a"), ("2026-01-01/inside.txt", "i"), ("desktop.ini", "[source icon]")],
        );
        tree(dest, &[("desktop.ini", "[destination icon]")]);
        tree(&dest.join(IN_PROGRESS), &[("half.txt", "h")]);
        mark(dest).await;

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["2026-01-01", "a.txt", "desktop.ini"]);
        assert_eq!(std::fs::read(dest.join("desktop.ini")).unwrap(), b"[destination icon]");
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_resumed_turning_off_does_not_take_a_source_folder_for_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        // Interrupted after the other days were deleted and `2026-01-01` — a
        // source folder — had already come up.
        tree(dest, &[("2026-01-01/inside.txt", "i")]);
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        let marker = Marker {
            leaving: Some("2026-09-22".into()),
            cleared: true,
            ..Marker::default()
        };
        write_marker(dest, &marker).await.unwrap();

        leave(dest, &go()).await.unwrap();

        assert_eq!(std::fs::read(dest.join("2026-01-01/inside.txt")).unwrap(), b"i");
        assert_eq!(names_at(dest), ["2026-01-01", "a.txt"]);
    }

    #[tokio::test]
    async fn an_interrupted_first_move_goes_back_up_when_versions_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("a.txt", "a")]);
        tree(&dest.join(IN_PROGRESS), &[("b.txt", "b")]);

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["a.txt", "b.txt"]);
    }

    #[tokio::test]
    async fn an_interrupted_first_move_is_finished_by_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("a.txt", "a")]);
        tree(&dest.join(IN_PROGRESS), &[("b.txt", "b")]);

        prepare(dest, 30, d("2026-09-21"), true, &go()).await.unwrap();

        assert_eq!(names_at(&dest.join(IN_PROGRESS)), ["a.txt", "b.txt"]);
        assert!(read_marker(dest).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn turning_on_again_mid_way_finishes_turning_off_first() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-21"), &[("old.txt", "old")]);
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        let marker = Marker { leaving: Some("2026-09-22".into()), ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();

        let plan = prepare(dest, 30, d("2026-09-23"), true, &go()).await.unwrap();

        assert!(!dest.join("2026-09-21").exists(), "the confirmed deletion is carried out");
        assert_eq!(plan, in_progress_on(dest, "2026-09-23"));
        assert_eq!(names_at(&dest.join(IN_PROGRESS)), ["a.txt"]);
    }

    /// Explorer showing a folder of the backup holds it open, and renaming it
    /// fails. The destination fails with no marker, and the next run carries
    /// the move on.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_folder_held_open_stops_the_first_move_and_the_next_run_finishes_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("busy/x.txt", "x"), ("a.txt", "a")]);
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: needed to open a folder
            .open(dest.join("busy"))
            .unwrap();

        assert!(prepare(dest, 30, d("2026-09-21"), true, &go()).await.is_err());
        assert_eq!(read_marker(dest).await.unwrap(), None);

        drop(held);
        prepare(dest, 30, d("2026-09-21"), true, &go()).await.unwrap();
        assert!(dest.join(IN_PROGRESS).join("busy/x.txt").exists());
        assert!(read_marker(dest).await.unwrap().is_some());
    }

    #[test]
    fn only_a_plain_date_names_a_snapshot() {
        assert_eq!(parse_day("2026-09-23"), NaiveDate::from_ymd_opt(2026, 9, 23));
        for name in [
            "2026-9-23",
            "2026-09-23 (copy)",
            "2026-02-30",
            "+2026-09-23",
            "Photos",
            IN_PROGRESS,
        ] {
            assert_eq!(parse_day(name), None, "{name}");
        }
        assert_eq!(day_name(d("2026-01-05")), "2026-01-05");
    }

    #[test]
    fn reserved_names_are_driveby_s_own_whatever_the_case() {
        assert!(is_reserved_name(".driveby-snapshots"));
        assert!(is_reserved_name(".DriveBy-In-Progress"));
        assert!(!is_reserved_name("driveby-snapshots"));
        assert!(!is_reserved_name("Photos"));
    }

    #[tokio::test]
    async fn list_finds_only_dated_folders_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["2026-09-23", "2026-09-21", IN_PROGRESS, "Photos"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("2026-09-22"), b"a file, not a snapshot").unwrap();
        std::fs::write(dir.path().join(MARKER), b"{}").unwrap();

        let names: Vec<String> = list(dir.path()).await.unwrap().iter().map(Snapshot::name).collect();

        assert_eq!(names, ["2026-09-21", "2026-09-23"]);
    }

    #[tokio::test]
    async fn the_marker_round_trips_and_its_absence_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(dir.path()).await.unwrap(), None);

        let marker = Marker { leaving: Some("2026-09-23".into()), cleared: true, ..Marker::default() };
        write_marker(dir.path(), &marker).await.unwrap();
        assert_eq!(read_marker(dir.path()).await.unwrap(), Some(marker));

        remove_marker(dir.path()).await.unwrap();
        assert_eq!(read_marker(dir.path()).await.unwrap(), None);
        remove_marker(dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn a_marker_that_does_not_parse_still_marks_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MARKER), b"not json").unwrap();
        assert_eq!(read_marker(dir.path()).await.unwrap(), Some(Marker::default()));
    }

    #[test]
    fn the_day_written_never_goes_back_before_the_newest_snapshot() {
        let root = Path::new("/b");
        let snaps = vec![snap(root, "2026-09-20"), snap(root, "2026-09-23")];
        assert_eq!(effective_day(d("2026-09-24"), &snaps), d("2026-09-24"));
        assert_eq!(effective_day(d("2026-09-21"), &snaps), d("2026-09-23"));
        assert_eq!(effective_day(d("2026-09-21"), &[]), d("2026-09-21"));
    }

    #[test]
    fn retention_keeps_the_window_and_always_the_newest() {
        let root = Path::new("/b");
        let snaps: Vec<Snapshot> = ["2026-09-01", "2026-09-15", "2026-09-16", "2026-09-23"]
            .iter()
            .map(|s| snap(root, s))
            .collect();
        let gone: Vec<String> =
            expired(&snaps, d("2026-09-23"), 7).iter().map(Snapshot::name).collect();
        assert_eq!(gone, ["2026-09-01", "2026-09-15"]);

        let stale = vec![snap(root, "2026-01-01")];
        assert!(expired(&stale, d("2026-09-23"), 7).is_empty(), "the last day always stays");
    }

    #[test]
    fn eviction_never_offers_the_newest() {
        let root = Path::new("/b");
        let snaps = vec![
            snap(root, "2026-09-21"),
            snap(root, "2026-09-22"),
            snap(root, "2026-09-23"),
        ];
        let names: Vec<String> = evictable(&snaps).iter().map(Snapshot::name).collect();
        assert_eq!(names, ["2026-09-21", "2026-09-22"]);
        assert!(evictable(&snaps[2..]).is_empty());
    }

    #[tokio::test]
    async fn remove_tree_takes_the_tree_and_leaves_what_it_shared() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept.txt");
        let old = dir.path().join("2026-09-01");
        std::fs::write(&kept, b"kept").unwrap();
        tree(&old, &[("a/own.txt", "own")]);
        std::fs::create_dir_all(old.join("a/b")).unwrap();
        std::fs::hard_link(&kept, old.join("a/b/linked.txt")).unwrap();

        remove_tree(&old, &go()).await.unwrap();

        assert!(!old.exists());
        assert_eq!(std::fs::read(&kept).unwrap(), b"kept");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn remove_tree_leaves_the_readonly_bit_on_what_it_shared() {
        use crate::fsutil::read_attrs;
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept.txt");
        let old = dir.path().join("2026-09-01");
        std::fs::write(&kept, b"kept").unwrap();
        crate::fsutil::apply_attrs(&kept, 0x1);
        std::fs::create_dir_all(old.join("Icon")).unwrap();
        std::fs::hard_link(&kept, old.join("Icon/linked.txt")).unwrap();
        crate::fsutil::apply_attrs(&old.join("Icon"), 0x1);

        remove_tree(&old, &go()).await.unwrap();

        assert!(!old.exists(), "a +R folder goes too");
        assert_ne!(read_attrs(&kept).unwrap() & 0x1, 0, "the shared file lost its +R");
        crate::fsutil::clear_readonly(&kept);
    }

    #[tokio::test]
    async fn clone_tree_links_every_file_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("2026-09-22");
        let into = dir.path().join(IN_PROGRESS);
        tree(&from, &[("a.txt", "a"), ("sub/b.txt", "b"), ("sub/b.txt.driveby-tmp", "half")]);
        std::fs::create_dir_all(from.join("sub/empty")).unwrap();
        // A resumed clone: one entry is already there, and stays as it is.
        tree(&into, &[("a.txt", "already synced")]);

        let stats = clone_tree(&from, &into, &go()).await.unwrap();

        assert_eq!(stats, CloneStats { linked: 1, copied: 0 });
        assert!(crate::fsutil::same_file(&from.join("sub/b.txt"), &into.join("sub/b.txt")));
        assert_eq!(std::fs::read(into.join("a.txt")).unwrap(), b"already synced");
        assert!(into.join("sub/empty").is_dir(), "an empty folder is part of the day too");
        assert!(!into.join("sub/b.txt.driveby-tmp").exists());
    }

    #[tokio::test]
    async fn a_stopped_clone_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("2026-09-22");
        tree(&from, &[("a.txt", "a")]);
        let stopped = go();
        stopped.cancel();
        assert!(clone_tree(&from, &dir.path().join(IN_PROGRESS), &stopped).await.is_err());
    }

    #[tokio::test]
    async fn move_entries_renames_skips_and_reports_what_it_could_not_move() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let into = root.join(IN_PROGRESS);
        tree(root, &[("Photos/cat.jpg", "cat"), ("notes.txt", "notes"), ("desktop.ini", "[icon]")]);
        tree(root, &[("clash", "root copy")]);
        std::fs::create_dir_all(into.join("clash")).unwrap();

        let moved = move_entries(root, &into, &[IN_PROGRESS, "desktop.ini"]).await.unwrap();

        assert_eq!(moved.moved, 2);
        assert_eq!(moved.blocked, ["clash"]);
        assert_eq!(std::fs::read(into.join("Photos/cat.jpg")).unwrap(), b"cat");
        assert!(root.join("desktop.ini").exists(), "the destination's own icon stays");
        assert_eq!(std::fs::read(root.join("clash")).unwrap(), b"root copy");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_tree_never_follows_a_link_out_of_the_tree() {
        use std::os::unix::fs as unix_fs;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        let outside_file = outside.join("file.txt");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&outside_file, b"outside").unwrap();

        let tree_root = dir.path().join("tree");
        std::fs::create_dir_all(&tree_root).unwrap();
        unix_fs::symlink(&outside, tree_root.join("link")).unwrap();

        remove_tree(&tree_root, &go()).await.unwrap();

        assert!(!tree_root.exists(), "tree is removed");
        assert!(outside.exists(), "target dir outside tree still exists");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside", "target file unchanged");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn clone_tree_leaves_links_out() {
        use std::os::unix::fs as unix_fs;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        let outside_file = outside.join("file.txt");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&outside_file, b"outside").unwrap();

        let from = dir.path().join("from");
        std::fs::create_dir_all(&from).unwrap();
        unix_fs::symlink(&outside, from.join("link")).unwrap();

        let into = dir.path().join("into");
        clone_tree(&from, &into, &go()).await.unwrap();

        assert!(!into.join("link").exists(), "symlink is not cloned");
        assert!(outside.exists(), "target dir outside tree still exists");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside", "target file unchanged");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn remove_tree_never_follows_a_junction_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        let outside_file = outside.join("file.txt");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&outside_file, b"outside").unwrap();

        let tree_root = dir.path().join("tree");
        std::fs::create_dir_all(&tree_root).unwrap();
        let junction = tree_root.join("junction");

        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", junction.to_str().unwrap(), outside.to_str().unwrap()])
            .status()
            .expect("mklink command failed");
        assert!(status.success(), "junction creation failed");

        remove_tree(&tree_root, &go()).await.unwrap();

        assert!(!tree_root.exists(), "tree is removed");
        assert!(outside.exists(), "target dir outside tree still exists");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside", "target file unchanged");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn clone_tree_leaves_junctions_out() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        let outside_file = outside.join("file.txt");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&outside_file, b"outside").unwrap();

        let from = dir.path().join("from");
        std::fs::create_dir_all(&from).unwrap();
        let junction = from.join("junction");

        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", junction.to_str().unwrap(), outside.to_str().unwrap()])
            .status()
            .expect("mklink command failed");
        assert!(status.success(), "junction creation failed");

        let into = dir.path().join("into");
        clone_tree(&from, &into, &go()).await.unwrap();

        assert!(!into.join("junction").exists(), "junction is not cloned");
        assert!(outside.exists(), "target dir outside tree still exists");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside", "target file unchanged");
    }
}
