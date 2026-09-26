//! Daily versions: a destination that keeps one whole snapshot per day, every
//! file unchanged since the day before hard-linked to that day's copy.
//!
//! ```text
//! <dest>/.driveby-snapshots      the marker: its presence makes this layout
//! <dest>/2026-09-21/             a committed snapshot, the whole tree
//! <dest>/.driveby-in-progress/   the snapshot being built; resumable
//! <dest>/.driveby-deleting-2026-09-01/   a day being deleted; no longer listed
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
/// What a day is renamed to, after this prefix, while it is being deleted.
pub(crate) const DELETING: &str = ".driveby-deleting-";

/// Names Driveby owns at a destination root. A source must not bring any of
/// them there: another destination's marker, mirrored onto this root, would
/// make it read as daily versions, and a folder named like a day being
/// deleted would be deleted.
pub(crate) fn is_reserved_name(name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case(MARKER)
        || name.eq_ignore_ascii_case(IN_PROGRESS)
        || is_being_discarded(name)
}

/// A day whose deletion has started (`discard_day`).
fn is_being_discarded(name: &str) -> bool {
    starts_with_ignore_case(name, DELETING)
}

/// Root entries that belong to the operating system, not to the backup:
/// Windows' restore points and recycle bins, ext4's `lost+found`, macOS's
/// trash, search index, event log and version store. At the root of a whole
/// drive they sit beside the backup, and the process may not rename them —
/// a first move that tried would fail the destination on every run. So they
/// stay where they are, and are never part of a day.
const SYSTEM_ENTRIES: [&str; 9] = [
    "System Volume Information",
    "$RECYCLE.BIN",
    "RECYCLER",
    "lost+found",
    ".Trashes",
    ".Spotlight-V100",
    ".fseventsd",
    ".TemporaryItems",
    ".DocumentRevisions-V100",
];

/// One of `SYSTEM_ENTRIES`, whatever the case, or a Linux per-user trash
/// (`.Trash-1000`).
fn is_system_entry(name: &str) -> bool {
    SYSTEM_ENTRIES.iter().any(|s| s.eq_ignore_ascii_case(name))
        || starts_with_ignore_case(name, ".Trash-")
}

/// Files a file manager writes into a folder it shows: its icon, its view
/// settings, its thumbnails. Browsing the days puts them at the destination
/// root, so one there says nothing about the backup, and a day's own copy is
/// the source's, which the mirror copies again.
const FOLDER_CLUTTER: [&str; 3] = ["desktop.ini", ".DS_Store", "Thumbs.db"];

fn is_folder_clutter(name: &str) -> bool {
    FOLDER_CLUTTER.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// What the first move leaves at the root: Driveby's own folders, the
/// operating system's entries and folder clutter.
fn stays_at_root(name: &str) -> bool {
    name.eq_ignore_ascii_case(IN_PROGRESS)
        || is_being_discarded(name)
        || is_system_entry(name)
        || is_folder_clutter(name)
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct Marker {
    pub(crate) version: u32,
    /// Set only while versions are being turned off: the snapshot that is
    /// becoming the mirror again, and `.driveby-in-progress` once it has been
    /// set aside there.
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
    let path = destination.join(MARKER);
    persist::write_json_atomic(&long_path(&path), marker).await?;
    // Hidden on Windows, as a dot-file is elsewhere: a stray file at the root
    // of a backup drive invites deleting, and without it every day reads as
    // a source folder a mirror prune deletes. A no-op elsewhere.
    crate::fsutil::apply_attrs(&path, 0x2 /*HIDDEN*/);
    Ok(())
}

/// Remove the marker; nothing to do when it is already gone.
pub(crate) async fn remove_marker(destination: &Path) -> Result<()> {
    match fs::remove_file(long_path(&destination.join(MARKER))).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// The folder a turning-off is bringing up, as the marker names it:
/// `.driveby-in-progress` or a day, and nothing else. The marker is a file
/// anyone can edit, and `leave` renames what it names into the destination
/// and brings its contents up to the root, where the mirror prune deletes
/// what the source does not have: an absolute path, or `..`, would reach
/// outside the destination.
fn leaving_of(destination: &Path, marker: &Marker) -> Result<Option<String>> {
    match marker.leaving.as_deref() {
        None => Ok(None),
        Some(name) if name == IN_PROGRESS || parse_day(name).is_some() => {
            Ok(Some(name.to_string()))
        }
        Some(_) => Err(anyhow!(
            "The versions marker at {} names a folder Driveby did not make; nothing was changed",
            destination.join(MARKER).display()
        )),
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

/// What is at `path`, the link itself rather than what it points to:
/// `Ok(true)` for a folder, `Ok(false)` for nothing there — or a file, which
/// is no folder of Driveby's either.
///
/// A symlink or a junction is an error. One named like a day or
/// `.driveby-in-progress` would have Driveby write, move and delete through
/// it, in a folder that can be anywhere. So is a path that cannot be looked
/// at: answering "not there" would, say, remove the marker while a day is
/// still in it.
pub(crate) async fn folder_state(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(long_path(path)).await {
        // std reports a Windows junction as a symlink too.
        Ok(meta) if meta.file_type().is_symlink() => Err(link_refused(path)),
        Ok(meta) => Ok(meta.is_dir()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("look at {}", path.display())),
    }
}

fn link_refused(path: &Path) -> anyhow::Error {
    anyhow!("{} is a link, not a folder Driveby made; nothing was changed", path.display())
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

/// Delete a tree Driveby owns — a day `discard_day` set aside, a stale
/// `.driveby-in-progress` — without touching the attributes of the files it
/// shares with other snapshots.
///
/// Files go through `remove_link_safe`: prune's clear-then-delete would strip
/// `+R` from every snapshot sharing the file. A symlink is removed as a link
/// and never followed. Directories are never shared, so their own `+R` (a
/// custom folder icon) is cleared before they go, deepest first. A `root`
/// that is itself a link is refused: listing it would list the folder it
/// points to.
///
/// One hop onto the blocking pool for the whole tree: a snapshot is as large
/// as the backup, and a hop per file would cost more than the work.
pub(crate) async fn remove_tree(root: &Path, token: &CancellationToken) -> Result<()> {
    let (root, token) = (root.to_path_buf(), token.clone());
    blocking(move || remove_tree_sync(&root, &token)).await
}

fn remove_tree_sync(root_path: &Path, token: &CancellationToken) -> Result<()> {
    let root = long_path(root_path);
    let meta = std::fs::symlink_metadata(&root)
        .with_context(|| format!("remove {}", root_path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(link_refused(root_path));
    }
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

/// Delete a day — retention, eviction, turning off. It is renamed out of the
/// list first (`.driveby-deleting-<day>` is not a date), then removed: a Stop
/// or a crash part-way leaves a folder nothing offers as a day, rather than a
/// day missing some of its files, and the next run finishes the deletion.
pub(crate) async fn discard_day(
    destination: &Path,
    snapshot: &Snapshot,
    token: &CancellationToken,
) -> Result<()> {
    let aside = destination.join(format!("{DELETING}{}", snapshot.name()));
    fs::rename(long_path(&snapshot.path), long_path(&aside))
        .await
        .with_context(|| format!("set the version of {} aside to delete it", snapshot.name()))?;
    remove_tree(&aside, token).await
}

/// Finish the deletions a Stop or a crash cut short: every folder at the root
/// named `.driveby-deleting-…`. Links are left alone, as `list` leaves them.
async fn finish_discards(destination: &Path, token: &CancellationToken) -> Result<()> {
    for name in entry_names(destination).await {
        let path = destination.join(&name);
        if is_being_discarded(&name) && folder_state(&path).await? {
            info!(dest = %destination.display(), "finishing the deletion of {}", name);
            remove_tree(&path, token).await?;
        }
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

/// Rename every entry of `from` into `into`, except the names `skip` answers
/// true for. Renames only, on one volume: instant, no room needed, nothing
/// deleted.
///
/// A name already taken in `into` is left where it is and reported. That is
/// asked again right before each rename, because `fs::rename` replaces an
/// existing file on Windows, and what it would replace is a copy this step
/// promises to keep. A rename that fails is an error naming the entry; the
/// caller stops there, and the next run finds the rest where it was.
pub(crate) async fn move_entries(
    from: &Path,
    into: &Path,
    skip: impl Fn(&str) -> bool,
) -> Result<Moved> {
    let mut report = Moved::default();
    // Listed before anything moves: renaming out of a directory while its
    // listing is still being read can skip entries.
    for name in entry_names(from).await {
        if skip(&name) {
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
    // A day whose deletion was cut short is already off the list; its room
    // is this run's.
    finish_discards(destination, token).await?;
    let in_progress = destination.join(IN_PROGRESS);
    // Before anything moves: a link there would take the first day, or
    // today's clone, wherever it points.
    folder_state(&in_progress).await?;
    // Turned off, then on again before turning off had finished: the user
    // confirmed deleting those versions, so that is finished first.
    if let Some(m) = &marker {
        if leaving_of(destination, m)?.is_some() {
            leave(destination, token).await?;
            marker = None;
        }
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

    if marker.is_none() {
        // The first run with versions: the mirror at the root becomes the
        // first snapshot, by renaming. The marker goes last, so a move that
        // stops half-way is simply carried on by the next run.
        fs::create_dir_all(long_path(&in_progress))
            .await
            .with_context(|| format!("create {}", in_progress.display()))?;
        let moved = move_entries(destination, &in_progress, stays_at_root).await?;
        // The marker says the root holds nothing but days. Written over a root
        // the move did not empty — a name already taken in the first day, a
        // listing cut short — it would strand mirror entries where nothing
        // looks again, and one named like a date would read as a day.
        let left = leftovers_at_root(destination).await?;
        if !left.is_empty() {
            return Err(anyhow!(
                "Could not move everything at {} into the first version; still at the root: {}",
                destination.display(),
                left.join(", ")
            ));
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
    let today = destination.join(&name);
    // Asked before retention deletes anything: a link named like today would
    // have the run write through it.
    let today_there = folder_state(&today).await?;
    for old in expired(&snapshots, day, keep_days) {
        info!(dest = %destination.display(), "deleting the version of {}", old.name());
        discard_day(destination, &old, token).await?;
    }
    if today_there {
        // A later run on the same day updates that day in place, and a
        // `.driveby-in-progress` beside it can only be a leftover.
        if folder_state(&in_progress).await? {
            remove_tree(&in_progress, token).await?;
        }
        return Ok(Plan::Snapshot { target: today, day: name, in_progress: false });
    }
    fs::create_dir_all(long_path(&in_progress))
        .await
        .with_context(|| format!("create {}", in_progress.display()))?;
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
/// before anything is deleted, `cleared` before anything moves, and `leaving`
/// again once the day is set aside, before anything comes up. So a run that
/// stops half-way knows which folder is coming up, and never takes a source
/// folder named like a date for a snapshot.
pub(crate) async fn leave(destination: &Path, token: &CancellationToken) -> Result<()> {
    let in_progress = destination.join(IN_PROGRESS);
    let Some(mut marker) = read_marker(destination).await? else {
        if folder_state(&in_progress).await? {
            come_up(destination, &in_progress, token).await?;
        }
        return Ok(());
    };
    // Both refused before anything is deleted or moved.
    let leaving = leaving_of(destination, &marker)?;
    let in_progress_there = folder_state(&in_progress).await?;
    finish_discards(destination, token).await?;
    let leaving = match leaving {
        Some(name) => name,
        None => {
            let name = match list(destination).await?.last() {
                Some(newest) => newest.name(),
                None if in_progress_there => IN_PROGRESS.to_string(),
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
                discard_day(destination, &snapshot, token).await?;
            }
        }
        if leaving != IN_PROGRESS && folder_state(&in_progress).await? {
            remove_tree(&in_progress, token).await?;
        }
        marker.cleared = true;
        write_marker(destination, &marker).await?;
    }
    // The day coming up is set aside under `.driveby-in-progress` first: a
    // source folder named like the day itself would otherwise collide with the
    // day's own folder at the root and block turning off for ever, and no
    // source can bring the reserved name. The marker says so before anything
    // comes up: from then on a folder at the root named like the day may be a
    // source folder that already came up, and is never set aside.
    if leaving != IN_PROGRESS {
        let day = destination.join(&leaving);
        match (folder_state(&day).await?, folder_state(&in_progress).await?) {
            (true, true) => return Err(set_aside_clash(destination, &leaving)),
            (true, false) => {
                fs::rename(long_path(&day), long_path(&in_progress))
                    .await
                    .with_context(|| format!("set the version of {} aside", leaving))?;
            }
            // Set aside by a run that stopped before the marker said so — or
            // nothing is left to bring up.
            (false, _) => {}
        }
        marker.leaving = Some(IN_PROGRESS.to_string());
        write_marker(destination, &marker).await?;
    }
    if folder_state(&in_progress).await? {
        come_up(destination, &in_progress, token).await?;
    }
    info!(
        dest = %destination.display(),
        "daily versions turned off; {} is the backup again",
        leaving
    );
    remove_marker(destination).await
}

/// The day a turning-off names and `.driveby-in-progress` both there, which
/// no step leaves behind: which of the two is coming back cannot be told.
fn set_aside_clash(destination: &Path, leaving: &str) -> anyhow::Error {
    anyhow!(
        "{} and {} are both at {}, and only one of them can become the backup again; \
         nothing was changed",
        leaving,
        IN_PROGRESS,
        destination.display()
    )
}

/// Move `from`'s entries up to `destination`, then remove the emptied folder.
async fn come_up(destination: &Path, from: &Path, token: &CancellationToken) -> Result<()> {
    let moved = move_entries(from, destination, |_| false).await?;
    for name in &moved.blocked {
        // The root's own folder clutter stays (its icon, its view settings):
        // the day's copy is the source's, which the mirror copies again. So do
        // the system's own folders, which a day only holds when its source was
        // a whole drive.
        if is_folder_clutter(name) || is_system_entry(name) {
            drop_entry(&from.join(name), token).await?;
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

/// Remove one entry of a day: a folder as a whole tree, a file as one link.
async fn drop_entry(path: &Path, token: &CancellationToken) -> Result<()> {
    if folder_state(path).await? {
        return remove_tree(path, token).await;
    }
    let copy = path.to_path_buf();
    blocking(move || remove_link_safe(&copy))
        .await
        .with_context(|| format!("remove {}", path.display()))
}

/// What the first move left at the destination root, besides what may stay
/// there: what `stays_at_root` names, and the scratch files of an
/// interrupted marker write or link probe. Symlinks are left alone by the
/// move, and here too. Unlike `backup::entry_names`, a listing error is an
/// error: "could not tell" must not read as "empty".
async fn leftovers_at_root(destination: &Path) -> Result<Vec<String>> {
    let root = long_path(destination);
    let mut entries = fs::read_dir(&root)
        .await
        .with_context(|| format!("list {}", root.display()))?;
    let mut left = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("list {}", root.display()))?
    {
        let kind = entry.file_type().await.with_context(|| format!("list {}", root.display()))?;
        if kind.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let stays = stays_at_root(&name)
            || name.starts_with(".driveby-link-probe")
            || (name.starts_with(".driveby-snapshots.") && name.ends_with(".tmp"));
        if !stays {
            left.push(name);
        }
    }
    left.sort();
    Ok(left)
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

/// The tree a run would start from, for the preview. The preview writes
/// nothing, so it cannot move or clone anything; it looks where the run would
/// find the files instead: the snapshot a turning-off is bringing up (decided
/// first, when the marker has `leaving` set), then today's day, else the
/// newest one (today's is cloned from it), else `.driveby-in-progress`, else
/// the root. A mirror, or a destination whose versions are about to begin, is
/// its own root.
pub(crate) async fn preview_base(
    destination: &Path,
    versions: bool,
    clock: NaiveDate,
) -> Result<PathBuf> {
    let Some(marker) = read_marker(destination).await? else {
        return Ok(destination.to_path_buf());
    };
    // Turning off: the day coming up is where the run starts — under its own
    // name, or set aside under `.driveby-in-progress` by `leave`, or already
    // back at the root — in the states `leave` goes through. The day list is
    // not consulted here, and once the marker names `.driveby-in-progress`
    // no folder named like a date is either: a source folder named like one
    // may already have come up to the root.
    let in_progress = destination.join(IN_PROGRESS);
    if let Some(leaving) = leaving_of(destination, &marker)? {
        if leaving != IN_PROGRESS {
            let day = destination.join(&leaving);
            // Until the other days are cleared, `.driveby-in-progress` is a
            // stopped run's leftover that `leave` removes before anything else.
            if !marker.cleared {
                return Ok(if folder_state(&day).await? {
                    day
                } else {
                    destination.to_path_buf()
                });
            }
            match (folder_state(&day).await?, folder_state(&in_progress).await?) {
                (true, true) => return Err(set_aside_clash(destination, &leaving)),
                (true, false) => return Ok(day),
                (false, _) => {}
            }
        }
        return Ok(if folder_state(&in_progress).await? {
            in_progress
        } else {
            destination.to_path_buf()
        });
    }
    let snapshots = list(destination).await?;
    if versions {
        let today = destination.join(day_name(effective_day(clock, &snapshots)));
        if folder_state(&today).await? {
            return Ok(today);
        }
    }
    Ok(match snapshots.last() {
        Some(newest) => newest.path.clone(),
        None if folder_state(&in_progress).await? => in_progress,
        None => destination.to_path_buf(),
    })
}

/// The day that keeps what a run leaves out of the version it writes: the
/// newest committed day before that version's own day that the run's
/// retention keeps. None when there is none — a first versioned run moves the
/// root into its first day, a turning-off brings one day up, a mirror has no
/// days — and what the run leaves out is then deleted for good.
pub(crate) async fn earlier_day(
    destination: &Path,
    keep_days: u32,
    clock: NaiveDate,
) -> Result<Option<PathBuf>> {
    let Some(marker) = read_marker(destination).await? else {
        return Ok(None);
    };
    if leaving_of(destination, &marker)?.is_some() {
        return Ok(None);
    }
    let snapshots = list(destination).await?;
    let day = effective_day(clock, &snapshots);
    let expired = expired(&snapshots, day, keep_days);
    Ok(snapshots
        .iter()
        .rev()
        .find(|s| s.date < day && !expired.iter().any(|e| e.date == s.date))
        .map(|s| s.path.clone()))
}

/// Why a destination whose versions are being turned off cannot be restored:
/// its data is split between a day and the root until the next run ends it.
pub(crate) fn turning_off(destination: &Path) -> anyhow::Error {
    anyhow!(
        "Daily versions are being turned off at {}: restore it once its next backup has finished",
        destination.display()
    )
}

/// One day a destination can be restored from.
#[derive(Serialize, Debug, PartialEq)]
pub(crate) struct DayInfo {
    pub(crate) name: String,
    pub(crate) path: String,
}

/// The days a destination with daily versions can be restored from, newest
/// first; none for a mirror, whose root is the backup. While versions are
/// being turned off, an error that says so. The path is built here so the
/// frontend never joins paths.
pub(crate) async fn restorable_days(destination: &Path) -> Result<Vec<DayInfo>> {
    let Some(marker) = read_marker(destination).await? else {
        return Ok(Vec::new());
    };
    // Turning off: the other days are being deleted, and the one coming back
    // may be half-way up to the root already.
    if leaving_of(destination, &marker)?.is_some() {
        return Err(turning_off(destination));
    }
    Ok(list(destination)
        .await?
        .into_iter()
        .rev()
        .map(|s| DayInfo { name: s.name(), path: s.path.to_string_lossy().to_string() })
        .collect())
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

    /// A link at `link` to the folder `target`: a junction on Windows, which
    /// needs no privilege, a symlink elsewhere.
    fn link_to(target: &Path, link: &Path) {
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(made.status.success(), "mklink /J failed: {made:?}");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// A folder beside the destination, holding one file no step may touch.
    fn outside(root: &Path) -> PathBuf {
        let outside = root.join("outside");
        tree(&outside, &[("keep.txt", "outside")]);
        outside
    }

    fn untouched(outside: &Path) {
        assert_eq!(names_at(outside), ["keep.txt"], "something was written through the link");
        assert_eq!(std::fs::read(outside.join("keep.txt")).unwrap(), b"outside");
    }

    /// At the root of a whole drive, the operating system's own folders sit
    /// beside the backup, and the process may not rename them. The first move
    /// leaves them where they are, and so it does a file manager's clutter.
    #[tokio::test]
    async fn the_first_move_leaves_the_system_s_own_folders_at_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(
            dest,
            &[
                ("System Volume Information/x", "x"),
                ("$RECYCLE.BIN/y", "y"),
                ("a.txt", "a"),
                (".DS_Store", "root"),
            ],
        );
        std::fs::create_dir(dest.join("lost+found")).unwrap();

        let plan = prepare(dest, 30, d("2026-09-21"), true, &go()).await.unwrap();

        assert_eq!(plan, in_progress_on(dest, "2026-09-21"));
        assert_eq!(names_at(&dest.join(IN_PROGRESS)), ["a.txt"]);
        assert_eq!(std::fs::read(dest.join("System Volume Information/x")).unwrap(), b"x");
        assert_eq!(std::fs::read(dest.join("$RECYCLE.BIN/y")).unwrap(), b"y");
        assert!(names_at(&dest.join("lost+found")).is_empty());
        assert_eq!(std::fs::read(dest.join(".DS_Store")).unwrap(), b"root");
        assert!(read_marker(dest).await.unwrap().is_some());
    }

    /// Browsing the days puts a file manager's clutter at the root. Turning
    /// off keeps the root's and drops the day's, which is the source's and
    /// comes back with the mirror; a system folder only a whole-drive source
    /// brought into the day goes the same way, as a whole tree.
    #[tokio::test]
    async fn turning_off_keeps_the_root_s_own_clutter_and_system_folders() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(
            &dest.join("2026-09-22"),
            &[
                ("a.txt", "a"),
                (".DS_Store", "day"),
                ("Thumbs.db", "day"),
                ("$RECYCLE.BIN/S-1/day.txt", "day"),
            ],
        );
        tree(
            dest,
            &[(".DS_Store", "root"), ("Thumbs.db", "root"), ("$RECYCLE.BIN/root.txt", "root")],
        );
        mark(dest).await;

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["$RECYCLE.BIN", ".DS_Store", "Thumbs.db", "a.txt"]);
        assert_eq!(std::fs::read(dest.join(".DS_Store")).unwrap(), b"root");
        assert_eq!(std::fs::read(dest.join("Thumbs.db")).unwrap(), b"root");
        assert_eq!(names_at(&dest.join("$RECYCLE.BIN")), ["root.txt"]);
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    #[test]
    fn the_system_s_own_root_entries_and_folder_clutter_are_known_whatever_the_case() {
        for name in [
            "System Volume Information",
            "$Recycle.Bin",
            "RECYCLER",
            "lost+found",
            ".Trashes",
            ".Spotlight-V100",
            ".fseventsd",
            ".TemporaryItems",
            ".DocumentRevisions-V100",
            ".Trash-1000",
        ] {
            assert!(is_system_entry(name), "{name}");
        }
        for name in ["Photos", "lost+found (copy)", ".Trash", "System", "desktop.ini"] {
            assert!(!is_system_entry(name), "{name}");
        }
        for name in ["desktop.ini", "Desktop.INI", ".DS_Store", "thumbs.db"] {
            assert!(is_folder_clutter(name), "{name}");
        }
        assert!(!is_folder_clutter("Thumbs.db.bak"));
    }

    /// A link named `.driveby-in-progress` beside a marker and a day: turning
    /// off would delete it as a leftover, and empty the folder it points to.
    #[tokio::test]
    async fn turning_off_refuses_a_link_named_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let (dest, outside) = (dir.path().join("dest"), outside(dir.path()));
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        mark(&dest).await;
        link_to(&outside, &dest.join(IN_PROGRESS));

        let err = leave(&dest, &go()).await.unwrap_err();

        assert!(err.to_string().contains("is a link"), "{err}");
        untouched(&outside);
        assert!(dest.join("2026-09-22/a.txt").exists());
    }

    /// Versions off and no marker: the link would come up as an interrupted
    /// first move, bringing what it points to into a root the mirror prunes.
    #[tokio::test]
    async fn a_mirror_run_refuses_a_link_named_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let (dest, outside) = (dir.path().join("dest"), outside(dir.path()));
        tree(&dest, &[("a.txt", "a")]);
        link_to(&outside, &dest.join(IN_PROGRESS));

        assert!(leave(&dest, &go()).await.is_err());

        untouched(&outside);
        assert!(!dest.join("keep.txt").exists());
    }

    #[tokio::test]
    async fn a_run_with_versions_refuses_a_link_named_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let (dest, outside) = (dir.path().join("dest"), outside(dir.path()));
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        mark(&dest).await;
        link_to(&outside, &dest.join(IN_PROGRESS));

        assert!(prepare(&dest, 30, d("2026-09-23"), true, &go()).await.is_err());
        untouched(&outside);

        // No marker yet: the first move would move the mirror into it.
        let first = dir.path().join("first");
        tree(&first, &[("a.txt", "a")]);
        link_to(&outside, &first.join(IN_PROGRESS));
        assert!(prepare(&first, 30, d("2026-09-23"), true, &go()).await.is_err());
        untouched(&outside);
        assert!(first.join("a.txt").exists());
        assert_eq!(read_marker(&first).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_link_named_like_today_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (dest, outside) = (dir.path().join("dest"), outside(dir.path()));
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        mark(&dest).await;
        link_to(&outside, &dest.join("2026-09-23"));

        assert!(prepare(&dest, 30, d("2026-09-23"), true, &go()).await.is_err());

        untouched(&outside);
    }

    #[tokio::test]
    async fn remove_tree_refuses_a_root_that_is_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let outside = outside(dir.path());
        let link = dir.path().join("2026-09-01");
        link_to(&outside, &link);

        assert!(remove_tree(&link, &go()).await.is_err());

        untouched(&outside);
    }

    /// The marker is a file anyone can edit. A `leaving` that names neither a
    /// day nor `.driveby-in-progress` is refused, never renamed in: what it
    /// names would come up to a root the mirror prunes.
    #[tokio::test]
    async fn a_marker_naming_a_folder_outside_the_destination_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (dest, outside) = (dir.path().join("dest"), outside(dir.path()));
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        for leaving in [outside.to_string_lossy().to_string(), "../outside".to_string()] {
            let marker = Marker { leaving: Some(leaving.clone()), ..Marker::default() };
            write_marker(&dest, &marker).await.unwrap();

            assert!(preview_base(&dest, false, d("2026-09-23")).await.is_err(), "{leaving}");
            assert!(restorable_days(&dest).await.is_err(), "{leaving}");
            assert!(prepare(&dest, 30, d("2026-09-23"), true, &go()).await.is_err(), "{leaving}");
            let err = leave(&dest, &go()).await.unwrap_err();

            assert!(err.to_string().contains("did not make"), "{err}");
            untouched(&outside);
            assert_eq!(names_at(&dest), [MARKER, "2026-09-22"], "{leaving}");
        }
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

    /// A source folder named like the day coming up — a photo import named by
    /// date — would collide with the day's own folder at the root. Turning off
    /// must still finish.
    #[tokio::test]
    async fn turning_off_finishes_when_the_source_has_a_folder_named_like_the_day() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-22"), &[("2026-09-22/x.txt", "x"), ("a.txt", "a")]);
        mark(dest).await;

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["2026-09-22", "a.txt"]);
        assert_eq!(std::fs::read(dest.join("2026-09-22/x.txt")).unwrap(), b"x");
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// `leaving` and `cleared` are on disk before anything moves, and the day
    /// set aside before anything comes up: a turning-off stopped half-way —
    /// here by a name already taken at the root — resumes without taking a
    /// source folder that already came up for a day.
    #[tokio::test]
    async fn turning_off_writes_its_progress_before_anything_moves() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-21"), &[("old.txt", "old")]);
        tree(
            &dest.join("2026-09-22"),
            &[("2030-01-01/inside.txt", "i"), ("a.txt", "day copy")],
        );
        tree(dest, &[("a.txt", "in the way")]);
        mark(dest).await;

        assert!(leave(dest, &go()).await.is_err());
        let marker = read_marker(dest).await.unwrap().expect("the marker stays until the end");
        assert_eq!(marker.leaving.as_deref(), Some(IN_PROGRESS), "the day was set aside");
        assert!(marker.cleared);
        assert!(!dest.join("2026-09-21").exists());

        std::fs::remove_file(dest.join("a.txt")).unwrap();
        leave(dest, &go()).await.unwrap();

        assert_eq!(std::fs::read(dest.join("2030-01-01/inside.txt")).unwrap(), b"i");
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"day copy");
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// A day is taken off the list before its deletion starts: a Stop part-way
    /// leaves a folder nothing offers as a day, and the next run finishes it.
    #[tokio::test]
    async fn a_stopped_discard_has_already_taken_the_day_off_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        for day in ["2026-09-01", "2026-09-22"] {
            tree(&dest.join(day), &[("a.txt", "a")]);
        }
        mark(dest).await;
        let stopped = go();
        stopped.cancel();

        let oldest = list(dest).await.unwrap().remove(0);
        assert!(discard_day(dest, &oldest, &stopped).await.is_err());

        let days: Vec<String> = list(dest).await.unwrap().iter().map(Snapshot::name).collect();
        assert_eq!(days, ["2026-09-22"]);
        let aside = dest.join(format!("{DELETING}2026-09-01"));
        assert!(aside.join("a.txt").exists(), "the Stop came before anything was removed");

        prepare(dest, 30, d("2026-09-22"), true, &go()).await.unwrap();
        assert!(!aside.exists());
    }

    #[tokio::test]
    async fn turning_off_finishes_an_interrupted_discard() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        tree(&dest.join(format!("{DELETING}2026-09-01")), &[("old.txt", "old")]);
        mark(dest).await;

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["a.txt"]);
    }

    /// Stopped while coming up, after a source folder named like the day had
    /// already reached the root. The marker said the day was set aside before
    /// anything came up, so the next run carries on from there instead of
    /// taking that folder for the day.
    #[tokio::test]
    async fn a_turning_off_stopped_after_a_date_named_folder_came_up_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-22"), &[("2026-09-22/x.txt", "x"), ("b.txt", "day copy")]);
        tree(dest, &[("b.txt", "in the way")]);
        mark(dest).await;

        assert!(leave(dest, &go()).await.is_err());
        let marker = read_marker(dest).await.unwrap().expect("the marker stays until the end");
        assert_eq!(marker.leaving.as_deref(), Some(IN_PROGRESS));
        assert!(dest.join("2026-09-22/x.txt").exists(), "the source folder came up");

        std::fs::remove_file(dest.join("b.txt")).unwrap();
        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["2026-09-22", "b.txt"]);
        assert_eq!(std::fs::read(dest.join("2026-09-22/x.txt")).unwrap(), b"x");
        assert_eq!(std::fs::read(dest.join("b.txt")).unwrap(), b"day copy");
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    fn set_aside(cleared: bool) -> Marker {
        Marker { leaving: Some(IN_PROGRESS.into()), cleared, ..Marker::default() }
    }

    /// The same state, written down: a folder at the root named like a date
    /// is never set aside once the marker names `.driveby-in-progress`.
    #[tokio::test]
    async fn a_resumed_turning_off_brings_up_what_is_left_of_the_day_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        write_marker(dest, &set_aside(true)).await.unwrap();
        tree(&dest.join(IN_PROGRESS), &[("a.txt", "a")]);
        tree(dest, &[("2026-09-22/x.txt", "x")]);

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["2026-09-22", "a.txt"]);
        assert_eq!(std::fs::read(dest.join("2026-09-22/x.txt")).unwrap(), b"x");
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// Stopped after everything came up, before the marker went: only the
    /// marker is left to remove, and a source folder named like a date at the
    /// root is not taken for the day.
    #[tokio::test]
    async fn a_turning_off_stopped_before_removing_the_marker_only_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        write_marker(dest, &set_aside(true)).await.unwrap();
        tree(dest, &[("2026-09-22/x.txt", "x"), ("a.txt", "a")]);

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["2026-09-22", "a.txt"]);
        assert_eq!(names_at(&dest.join("2026-09-22")), ["x.txt"]);
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// Stopped between setting the day aside and saying so in the marker.
    #[tokio::test]
    async fn a_day_set_aside_before_the_marker_said_so_comes_up() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        let marker =
            Marker { leaving: Some("2026-09-22".into()), cleared: true, ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();
        tree(&dest.join(IN_PROGRESS), &[("a.txt", "a")]);

        leave(dest, &go()).await.unwrap();

        assert_eq!(names_at(dest), ["a.txt"]);
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// The day the marker names and `.driveby-in-progress` both there: which
    /// one is coming back cannot be told, so neither is touched.
    #[tokio::test]
    async fn turning_off_stops_when_the_day_and_a_set_aside_copy_are_both_there() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        let marker =
            Marker { leaving: Some("2026-09-22".into()), cleared: true, ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        tree(&dest.join(IN_PROGRESS), &[("b.txt", "b")]);

        let err = leave(dest, &go()).await.unwrap_err();

        assert!(err.to_string().contains(IN_PROGRESS), "{err}");
        assert_eq!(names_at(dest), [IN_PROGRESS, MARKER, "2026-09-22"]);
        assert_eq!(names_at(&dest.join(IN_PROGRESS)), ["b.txt"]);
        assert_eq!(names_at(&dest.join("2026-09-22")), ["a.txt"]);
    }

    /// Turning off deletes the other days and brings one up: none of them is
    /// offered for restore meanwhile, and asking says why before a folder to
    /// restore into is picked.
    #[tokio::test]
    async fn no_day_is_offered_for_restore_while_versions_are_being_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        for day in ["2026-09-21", "2026-09-22"] {
            tree(&dest.join(day), &[("a.txt", "a")]);
        }
        let marker = Marker { leaving: Some("2026-09-22".into()), ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();

        let err = restorable_days(dest).await.unwrap_err();
        assert!(err.to_string().contains("being turned off"), "{err}");
    }

    /// The marker says the root holds nothing but days. When the move could
    /// not empty it, no marker is written and the destination fails, naming
    /// what is left.
    #[tokio::test]
    async fn a_first_move_that_leaves_something_behind_writes_no_marker() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(dest, &[("a.txt", "root copy")]);
        tree(&dest.join(IN_PROGRESS), &[("a.txt", "already moved")]);

        let err = prepare(dest, 30, d("2026-09-21"), true, &go()).await.unwrap_err();

        assert!(err.to_string().contains("a.txt"), "{err}");
        assert_eq!(read_marker(dest).await.unwrap(), None);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"root copy");
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
        assert!(is_reserved_name(".driveby-deleting-2026-09-01"));
        assert!(is_reserved_name(".Driveby-Deleting-anything"));
        assert!(!is_reserved_name(".driveby-deleting"));
        assert!(!is_reserved_name("driveby-snapshots"));
        assert!(!is_reserved_name("Photos"));
    }

    #[tokio::test]
    async fn the_days_to_restore_are_the_committed_ones_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        for day in ["2026-09-21", "2026-09-23"] {
            tree(&dest.join(day), &[("a.txt", "a")]);
        }
        tree(&dest.join(IN_PROGRESS), &[("a.txt", "a")]);
        mark(dest).await;

        let days = restorable_days(dest).await.unwrap();

        let names: Vec<&str> = days.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["2026-09-23", "2026-09-21"]);
        assert_eq!(days[0].path, dest.join("2026-09-23").to_string_lossy());

        let mirror = tempfile::tempdir().unwrap();
        tree(&mirror.path().join("2026-09-21"), &[("a.txt", "a")]);
        assert!(restorable_days(mirror.path()).await.unwrap().is_empty(), "a mirror has no days");
    }

    #[tokio::test]
    async fn the_preview_looks_where_the_run_will_start() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        assert_eq!(preview_base(dest, true, d("2026-09-23")).await.unwrap(), dest);

        tree(&dest.join("2026-09-21"), &[("a.txt", "a")]);
        mark(dest).await;
        let newest = dest.join("2026-09-21");
        assert_eq!(preview_base(dest, true, d("2026-09-23")).await.unwrap(), newest);
        assert_eq!(preview_base(dest, false, d("2026-09-23")).await.unwrap(), newest);

        tree(&dest.join("2026-09-23"), &[("a.txt", "a")]);
        let today = dest.join("2026-09-23");
        assert_eq!(preview_base(dest, true, d("2026-09-23")).await.unwrap(), today);
    }

    /// Mid turn-off, the preview follows the day coming up wherever `leave`
    /// has put it, and never mistakes a source folder named like a date that
    /// already came up for a day.
    #[tokio::test]
    async fn the_preview_follows_a_turning_off_wherever_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        let marker =
            Marker { leaving: Some("2026-09-22".into()), cleared: true, ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();
        tree(dest, &[("2030-01-01/inside.txt", "a source folder that came up")]);

        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        assert_eq!(
            preview_base(dest, false, d("2026-09-23")).await.unwrap(),
            dest.join("2026-09-22")
        );

        std::fs::rename(dest.join("2026-09-22"), dest.join(IN_PROGRESS)).unwrap();
        assert_eq!(
            preview_base(dest, false, d("2026-09-23")).await.unwrap(),
            dest.join(IN_PROGRESS)
        );

        std::fs::remove_dir_all(dest.join(IN_PROGRESS)).unwrap();
        assert_eq!(preview_base(dest, false, d("2026-09-23")).await.unwrap(), dest);

        // Set aside, and the marker says so: a folder named like the day at
        // the root is a source folder that came up, never the day.
        write_marker(dest, &set_aside(true)).await.unwrap();
        tree(dest, &[("2026-09-22/x.txt", "a source folder that came up")]);
        tree(&dest.join(IN_PROGRESS), &[("a.txt", "a")]);
        assert_eq!(
            preview_base(dest, false, d("2026-09-23")).await.unwrap(),
            dest.join(IN_PROGRESS)
        );
        std::fs::remove_dir_all(dest.join(IN_PROGRESS)).unwrap();
        assert_eq!(preview_base(dest, false, d("2026-09-23")).await.unwrap(), dest);
    }

    /// Until the other days are cleared, `.driveby-in-progress` is what a
    /// stopped run left behind and `leave` removes it first: the preview
    /// neither takes it for the day set aside nor fails on it.
    #[tokio::test]
    async fn the_preview_ignores_a_stopped_run_s_leftover_until_the_days_are_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        let marker = Marker { leaving: Some("2026-09-22".into()), ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);
        tree(&dest.join(IN_PROGRESS), &[("b.txt", "left by a stopped run")]);

        assert_eq!(
            preview_base(dest, false, d("2026-09-23")).await.unwrap(),
            dest.join("2026-09-22")
        );

        std::fs::remove_dir_all(dest.join("2026-09-22")).unwrap();
        assert_eq!(preview_base(dest, false, d("2026-09-23")).await.unwrap(), dest);
    }

    /// A marker that is there but cannot be read fails the destination: read
    /// as "no marker", the root would be taken for a mirror and every day
    /// moved or pruned.
    #[tokio::test]
    async fn a_marker_that_cannot_be_read_fails_the_destination_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        std::fs::create_dir(dest.join(MARKER)).unwrap();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);

        assert!(read_marker(dest).await.is_err());
        assert!(prepare(dest, 30, d("2026-09-23"), true, &go()).await.is_err());
        assert!(leave(dest, &go()).await.is_err());
        assert!(preview_base(dest, true, d("2026-09-23")).await.is_err());
        assert_eq!(names_at(dest), [MARKER, "2026-09-22"]);
        assert_eq!(names_at(&dest.join("2026-09-22")), ["a.txt"]);
    }

    /// One that can be read but not parsed still marks the layout: the day at
    /// the root is cloned from, not moved into a first day.
    #[tokio::test]
    async fn a_marker_that_cannot_be_parsed_still_marks_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        std::fs::write(dest.join(MARKER), b"not json").unwrap();
        tree(&dest.join("2026-09-22"), &[("a.txt", "a")]);

        assert_eq!(read_marker(dest).await.unwrap(), Some(Marker::default()));
        prepare(dest, 30, d("2026-09-23"), true, &go()).await.unwrap();
        assert_eq!(names_at(&dest.join("2026-09-22")), ["a.txt"]);
        assert_eq!(names_at(&dest.join(IN_PROGRESS)), ["a.txt"]);
    }

    /// Hidden on Windows, as a dot-file is elsewhere: deleting it by hand
    /// would hand every day to a mirror prune. Rewritten and removed all the
    /// same.
    #[cfg(windows)]
    #[tokio::test]
    async fn the_marker_is_hidden_on_windows_and_still_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        let hidden = |p: &Path| crate::fsutil::read_attrs(p).unwrap() & 0x2 != 0;

        mark(dest).await;
        assert!(hidden(&dest.join(MARKER)));

        let marker = Marker { leaving: Some("2026-09-22".into()), ..Marker::default() };
        write_marker(dest, &marker).await.unwrap();
        assert_eq!(read_marker(dest).await.unwrap(), Some(marker));
        assert!(hidden(&dest.join(MARKER)));

        remove_marker(dest).await.unwrap();
        assert_eq!(read_marker(dest).await.unwrap(), None);
    }

    /// What a run leaves out of the version it writes is kept by the day
    /// before that version — when there is one retention keeps.
    #[tokio::test]
    async fn the_day_before_keeps_what_a_run_leaves_out() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path();
        tree(&dest.join("2026-09-10"), &[("a.txt", "a")]);
        // No marker: a source folder named like a date; the first versioned
        // run moves the whole root into its first day.
        assert_eq!(earlier_day(dest, 30, d("2026-09-23")).await.unwrap(), None);

        mark(dest).await;
        tree(&dest.join("2026-09-21"), &[("a.txt", "a")]);
        let before = Some(dest.join("2026-09-21"));
        // Today's is cloned from the newest day.
        assert_eq!(earlier_day(dest, 30, d("2026-09-23")).await.unwrap(), before);
        // Today's is already there: the day before it.
        tree(&dest.join("2026-09-23"), &[("a.txt", "a")]);
        assert_eq!(earlier_day(dest, 30, d("2026-09-23")).await.unwrap(), before);
        // Unless this very run's retention deletes it.
        assert_eq!(earlier_day(dest, 1, d("2026-09-23")).await.unwrap(), None);

        let leaving = Marker { leaving: Some("2026-09-23".into()), ..Marker::default() };
        write_marker(dest, &leaving).await.unwrap();
        assert_eq!(earlier_day(dest, 30, d("2026-09-23")).await.unwrap(), None);
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

        let moved = move_entries(root, &into, stays_at_root).await.unwrap();

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

        link_to(&outside, &junction);

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

        link_to(&outside, &junction);

        let into = dir.path().join("into");
        clone_tree(&from, &into, &go()).await.unwrap();

        assert!(!into.join("junction").exists(), "junction is not cloned");
        assert!(outside.exists(), "target dir outside tree still exists");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside", "target file unchanged");
    }
}
