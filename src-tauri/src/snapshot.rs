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
use tracing::warn;

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
}
