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

use crate::fsutil::long_path;
use crate::persist;
use anyhow::{Context, Result};
use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        parse_day(s).unwrap()
    }

    fn snap(root: &Path, s: &str) -> Snapshot {
        Snapshot { date: d(s), path: root.join(s) }
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
}
