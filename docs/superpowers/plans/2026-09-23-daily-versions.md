# Daily Versions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a task keep one hard-linked snapshot of its backup per day, for a
chosen number of days, so a file can be taken back as it was on an earlier
day. This ships as Driveby 2.0.0.

**Architecture:** A new `src-tauri/src/snapshot.rs` owns the layout: the
marker, day folders, the in-progress folder, and the moves between a mirror
and snapshots. Each day starts as a hard-link clone of the day before (the
rsnapshot pattern). The existing pipeline in `backup.rs` then runs unchanged
on that clone. It gets one flag, which makes replacing and deleting leave the
Windows ReadOnly bit of shared files alone. The preview, restore and the
frontend read the new layout through two small entry points,
`snapshot::preview_base` and `snapshot::restorable_days`.

**Tech Stack:** Rust (Tauri 2, tokio, chrono 0.4.44, windows-sys 0.59),
React 18, vitest.

**Spec:** `docs/superpowers/specs/2026-09-23-daily-versions-design.md`.
Read it first. Every ordering decision below is argued there.

## Global Constraints

- The work happens on the local branch `feature/daily-versions`, in its own
  worktree. Each task ends by committing the files it touched there. Stage
  them by path, never with `git add -A` or `git add .`. Nothing is pushed or
  merged into master without the user's go-ahead: they try the branch in
  `npm run tauri dev` (Windows, then the Kali VM) first.
- Do not touch `src/main.jsx`, `src/styles.css` or `src/themes/sequoia.css`.
  The user has uncommitted work on them in the main checkout, and changing
  them here would conflict at merge.
- Never run `cargo fmt`: the crate is not rustfmt-clean. Hand-wrap new Rust
  lines to about 100 columns, and check with `awk 'length > 100' <file>`.
- Code, comments and English strings in English. French UI strings use the
  *vous* form, like the rest of `src/lib/i18n.js`.
- Reserved names: marker `.driveby-snapshots`, in-progress folder
  `.driveby-in-progress`. Day folders: exactly `YYYY-MM-DD`, local date.
- `keepVersionsDays`: absent or 0 means off; clamped to 1..=1000. The form
  offers 0, 7, 30, 90, 365.
- Marker JSON: `{"version":1,"leaving":null,"cleared":false}`.
- With versions off, the mirror path must behave byte-for-byte as today:
  `link_safe` is false and every existing test passes unchanged.
- Rust tests: `cargo test --manifest-path src-tauri/Cargo.toml <filter>`, run
  from the repo root. Frontend: `npm test`.
- Version 2.0.0: the CHANGELOG section is `## 2.0.0`. The version bump itself
  (`npm run bump-version 2.0.0`, then
  `cargo metadata --manifest-path src-tauri/Cargo.toml`) belongs to the
  release, not to this plan.

## Review Focus

The five conditions most likely to bite someone using this that no happy-path
test exercises. Each one's test is written into the task that owns the code.

1. **A folder open in Explorer during the first versioned run.** The rename
   fails with a sharing violation. The destination must fail with no marker
   written, and the next run must finish the move. → Task 5,
   `a_folder_held_open_stops_the_first_move_and_the_next_run_finishes_it`
   (Windows).
2. **A read-only file shared with yesterday's snapshot, replaced or deleted
   today.** Yesterday's copy must keep its content and its `+R`. → Task 7,
   `a_readonly_file_keeps_its_bit_in_yesterdays_version` (Windows).
3. **A source folder named like a date (`2026-01-01`) while versions are
   turned off, with a crash half-way.** It must never be deleted as if it were
   a snapshot. → Task 5,
   `a_resumed_turning_off_does_not_take_a_source_folder_for_a_day`.
4. **A clock that reads a day earlier than the newest snapshot.** The run
   writes into the newest snapshot and never creates an older-named one. →
   Task 5, `a_clock_behind_the_newest_day_writes_into_it`.
5. **A run stopped during the clone, then a run on a later day.** No day is
   named for the stopped run, and the later one finishes and commits. → Task
   7, `a_stopped_run_names_no_day_and_the_next_run_finishes_it`.

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/src/fsutil.rs` | Link-safe replace and remove, the hard-link probe, `finish_scratch`, and the `same_file` test helper. |
| `src-tauri/src/snapshot.rs` (new) | The layout: names, marker, listing, retention and eviction policy, tree operations (remove, clone, move), `prepare` / `leave` / `commit`, `preview_base`, `restorable_days`. |
| `src-tauri/src/backup.rs` | The task option, reserved names in the walk, room without credit and eviction, and wiring `snapshot` into `execute_one`. `RunCtx.link_safe` drives copy and prune. New outcome fields. |
| `src-tauri/src/preview.rs` | Counts against the tree the run starts from. `versions` flag per destination. |
| `src-tauri/src/restore.rs` | Refuses a snapshot root. |
| `src-tauri/src/main.rs` | `mod snapshot`, and the `list_snapshots` command. |
| `src/lib/task.js`, `src/lib/format.js` | `keepVersionsDays`, `VERSION_CHOICES`, the reserved-name check, and `formatDay`. |
| `src/components/ConfirmDialog.jsx` | Optional `choices`, which resolve with the picked value. |
| `src/components/NewTaskForm.jsx`, `src/components/Home.jsx` | The Versions select, and confirmation before versions are deleted. |
| `src/context/AppContext.jsx`, `src/lib/tauri.js` | The day picker in the restore flow, and the notification when versions are unavailable. |
| `src/components/History.jsx`, `src/components/BackupPreviewDialog.jsx` | The orange notes, the restore day, and the wording of deletions. |
| `src/lib/i18n.js` | English and French strings. |
| `README.md`, `CHANGELOG.md` | The user-facing description, and `## 2.0.0`. |

---

### Task 1: Link-safe replace and remove in fsutil

**Files:**
- Modify: `src-tauri/src/fsutil.rs`: `finish_copy` (around line 413), and a
  new section before `// Free space` (around line 316).
- Test: the `mod tests` block of `src-tauri/src/fsutil.rs`.

**Interfaces:**
- Consumes: `long_path`, `read_attrs`, `apply_attrs`, `clear_readonly` and
  `blocking`, all already in `fsutil.rs`.
- Produces:
  - `pub fn replace_link_safe(tmp: &Path, dest: &Path) -> std::io::Result<()>`
  - `pub fn remove_link_safe(path: &Path) -> std::io::Result<()>`
  - `pub fn hard_link_supported(dir: &Path) -> bool`
  - `pub async fn finish_scratch(src: PathBuf, tmp: PathBuf, mtime: Option<std::time::SystemTime>)`
  - `#[cfg(test)] pub(crate) fn same_file(a: &Path, b: &Path) -> bool`

Background, measured on NTFS with rustc 1.98 while planning:
- `std::fs::rename` over a `+R` hard link fails with "access denied".
- `FileRenameInfoEx` with the flags `0x1|0x2|0x40` replaces it and leaves the
  other link's content and `+R` intact.
- `std::fs::remove_file` deletes a `+R` link and leaves the other link's
  `+R`.
- Clearing `+R` on one link clears it on all of them.

- [ ] **Step 1: Write the failing tests**

Add at the end of `mod tests` in `src-tauri/src/fsutil.rs`:

```rust
    fn set_readonly(p: &Path, on: bool) {
        let mut perms = std::fs::metadata(p).unwrap().permissions();
        perms.set_readonly(on);
        std::fs::set_permissions(p, perms).unwrap();
    }

    /// Replacing today's link must leave yesterday's file exactly as it was:
    /// that is the whole of what a snapshot promises.
    #[test]
    fn replace_link_safe_replaces_only_this_link() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        let tmp = dir.path().join("today.txt.driveby-tmp");
        std::fs::write(&yesterday, b"old").unwrap();
        std::fs::hard_link(&yesterday, &today).unwrap();
        std::fs::write(&tmp, b"new").unwrap();

        replace_link_safe(&tmp, &today).unwrap();

        assert_eq!(std::fs::read(&today).unwrap(), b"new");
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
        assert!(!tmp.exists(), "the scratch file is what became today's copy");
        assert!(!same_file(&today, &yesterday));
    }

    /// On Windows ReadOnly belongs to the file, not the link: replacing a
    /// `+R` link the way `finish_copy` does — clear, then rename — strips the
    /// bit from yesterday's copy as well.
    #[cfg(windows)]
    #[test]
    fn replace_link_safe_leaves_the_other_links_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        let tmp = dir.path().join("today.txt.driveby-tmp");
        std::fs::write(&yesterday, b"old").unwrap();
        set_readonly(&yesterday, true);
        std::fs::hard_link(&yesterday, &today).unwrap();
        std::fs::write(&tmp, b"new").unwrap();

        replace_link_safe(&tmp, &today).unwrap();

        assert_eq!(std::fs::read(&today).unwrap(), b"new");
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
        assert_ne!(read_attrs(&yesterday).unwrap() & ATTR_READONLY, 0, "yesterday lost its +R");
        set_readonly(&yesterday, false);
    }

    #[test]
    fn remove_link_safe_removes_one_link() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        std::fs::write(&yesterday, b"old").unwrap();
        std::fs::hard_link(&yesterday, &today).unwrap();

        remove_link_safe(&today).unwrap();

        assert!(!today.exists());
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
    }

    #[cfg(windows)]
    #[test]
    fn remove_link_safe_leaves_the_other_links_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        std::fs::write(&yesterday, b"old").unwrap();
        set_readonly(&yesterday, true);
        std::fs::hard_link(&yesterday, &today).unwrap();

        remove_link_safe(&today).unwrap();

        assert!(!today.exists());
        assert_ne!(read_attrs(&yesterday).unwrap() & ATTR_READONLY, 0, "yesterday lost its +R");
        set_readonly(&yesterday, false);
    }

    #[test]
    fn a_local_folder_can_hard_link_and_keeps_no_probe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(hard_link_supported(dir.path()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the probe files must go");
    }

    #[test]
    fn same_file_tells_a_link_from_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        std::fs::copy(&a, &c).unwrap();
        assert!(same_file(&a, &b));
        assert!(!same_file(&a, &c));
    }

    /// `finish_scratch` stamps the scratch file as `finish_copy` does, and
    /// leaves the ReadOnly bit of the file it will replace alone — that bit is
    /// shared with earlier days.
    #[cfg(windows)]
    #[tokio::test]
    async fn finish_scratch_leaves_the_outgoing_files_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let tmp = dir.path().join("dest.txt.driveby-tmp");
        let dest = dir.path().join("dest.txt");
        std::fs::write(&src, b"source").unwrap();
        std::fs::write(&tmp, b"source").unwrap();
        std::fs::write(&dest, b"old").unwrap();
        set_readonly(&dest, true);
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);

        finish_scratch(src, tmp.clone(), Some(when)).await;

        assert_eq!(std::fs::metadata(&tmp).unwrap().modified().unwrap(), when);
        assert_ne!(read_attrs(&dest).unwrap() & ATTR_READONLY, 0);
        set_readonly(&dest, false);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml fsutil::`
Expected: compile errors, "cannot find function `replace_link_safe`" and
the same for `remove_link_safe`, `hard_link_supported`, `same_file` and
`finish_scratch`.

- [ ] **Step 3: Split `finish_copy` and add `finish_scratch`**

Replace the body of `finish_copy`, keeping its doc comment and signature:

```rust
pub async fn finish_copy(
    src: PathBuf,
    tmp: PathBuf,
    dest: PathBuf,
    mtime: Option<std::time::SystemTime>,
) {
    blocking(move || {
        stamp_scratch(&src, &tmp, mtime);
        clear_readonly(&dest);
    })
    .await
}

/// `finish_copy` for a scratch file that will replace a file shared with
/// earlier snapshots: the same stamping, and the read-only bit left on the
/// outgoing file, because it is theirs too. `replace_link_safe` does not need
/// it gone.
pub async fn finish_scratch(src: PathBuf, tmp: PathBuf, mtime: Option<std::time::SystemTime>) {
    blocking(move || stamp_scratch(&src, &tmp, mtime)).await
}

/// The source's mtime (when kept) and its kept attribute bits, onto the
/// scratch file.
fn stamp_scratch(src: &Path, tmp: &Path, mtime: Option<std::time::SystemTime>) {
    if let Some(t) = mtime {
        let _ = filetime::set_file_mtime(long_path(tmp), filetime::FileTime::from_system_time(t));
    }
    if let Some(attrs) = read_attrs(src) {
        apply_attrs(tmp, attrs);
    }
}
```

- [ ] **Step 4: Add the link-safe section**

Insert before the `// Free space` banner:

```rust
// ─────────────────────────────────────────────────────────────────────
// Hard links (daily versions)
// ─────────────────────────────────────────────────────────────────────

/// Rename `tmp` over `dest` without touching the attributes of the file
/// `dest` names.
///
/// With daily versions `dest` is usually a hard link shared with earlier
/// snapshots, and on Windows the ReadOnly bit belongs to the file, not to the
/// link: `finish_copy`'s `clear_readonly(dest)` strips it from every day that
/// shares the file. `std::fs::rename` will not replace a `+R` file at all.
/// `FileRenameInfoEx` with `FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE`
/// replaces it, atomically, and leaves the other links as they were (NTFS,
/// Windows 10 1809 and later — measured with rustc 1.98). Where the call is
/// not supported, fall back to clear-then-rename, and say so.
#[cfg(windows)]
pub fn replace_link_safe(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileRenameInfoEx, SetFileInformationByHandle, DELETE, FILE_RENAME_INFO,
    };
    // winbase.h. windows-sys 0.59 exports these only under
    // Win32_System_WindowsProgramming, which nothing else here needs.
    const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 0x1;
    const FILE_RENAME_FLAG_POSIX_SEMANTICS: u32 = 0x2;
    const FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE: u32 = 0x40;

    let attempt = || -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new().access_mode(DELETE).open(long_path(tmp))?;
        let name: Vec<u16> = long_path(dest).as_os_str().encode_wide().collect();
        let size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
        // u64 storage keeps the buffer aligned for the struct's HANDLE field.
        let mut buf = vec![0u64; size.div_ceil(8)];
        let info = buf.as_mut_ptr() as *mut FILE_RENAME_INFO;
        let ok = unsafe {
            (*info).Anonymous.Flags = FILE_RENAME_FLAG_REPLACE_IF_EXISTS
                | FILE_RENAME_FLAG_POSIX_SEMANTICS
                | FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE;
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
            SetFileInformationByHandle(
                file.as_raw_handle() as _,
                FileRenameInfoEx,
                info as *const _,
                size as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    match attempt() {
        Ok(()) => Ok(()),
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER:
        // an older Windows, or a filesystem without the Ex information class.
        Err(e) if matches!(e.raw_os_error(), Some(1) | Some(50) | Some(87)) => {
            warn!(
                "link-safe rename unsupported ({}); clearing ReadOnly on {} first",
                e,
                dest.display()
            );
            clear_readonly(dest);
            std::fs::rename(long_path(tmp), long_path(dest))
        }
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
pub fn replace_link_safe(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, dest)
}

/// Delete `path` without touching the attributes of the file it names.
///
/// std's `remove_file` already deletes a `+R` file on NTFS without clearing
/// the bit — it asks for `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE` —
/// so the other links to it keep theirs (measured with rustc 1.98). What
/// strips the bit is prune's `clear_readonly` *before* the delete. Only a
/// filesystem without that disposition class answers PermissionDenied, and
/// there clear-then-delete is the only way.
pub fn remove_link_safe(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(long_path(path)) {
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            clear_readonly(path);
            std::fs::remove_file(long_path(path))
        }
        other => other,
    }
}

/// Whether `dir`'s filesystem can hard-link: make a file, link it, remove
/// both. exFAT and FAT32 cannot, and neither can some network shares.
pub fn hard_link_supported(dir: &Path) -> bool {
    let a = dir.join(".driveby-link-probe");
    let b = dir.join(".driveby-link-probe-2");
    // Leftovers from a run killed mid-probe.
    let _ = std::fs::remove_file(long_path(&b));
    let _ = std::fs::remove_file(long_path(&a));
    if std::fs::write(long_path(&a), b"probe").is_err() {
        return false;
    }
    let linked = std::fs::hard_link(long_path(&a), long_path(&b)).is_ok();
    let _ = std::fs::remove_file(long_path(&b));
    let _ = std::fs::remove_file(long_path(&a));
    linked
}

/// Whether `a` and `b` are one file reached by two links.
#[cfg(test)]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
            return false;
        };
        x.dev() == y.dev() && x.ino() == y.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let id = |p: &Path| -> Option<(u32, u32, u32)> {
            let f = std::fs::File::open(long_path(p)).ok()?;
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) };
            (ok != 0).then_some((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
        };
        matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
    }
}
```

`warn` is imported only under `#[cfg(windows)]` at the top of `fsutil.rs`,
which is the only place `replace_link_safe` uses it. Nothing to change there.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml fsutil::`
Expected: PASS, including the existing `finish_copy_*` tests.

- [ ] **Step 6: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → every test passes.
Run: `awk 'length > 100' src-tauri/src/fsutil.rs` → no new line appears.
Commit the files this task touched on the branch.

---

### Task 2: snapshot.rs — names, marker, listing and policy

**Files:**
- Create: `src-tauri/src/snapshot.rs`
- Modify: `src-tauri/src/main.rs:3-13`: add `mod snapshot;` to the module
  list, alphabetically after `mod scheduler;`.

**Interfaces:**
- Consumes: `crate::fsutil::long_path` and
  `crate::persist::write_json_atomic(path, value) -> anyhow::Result<()>`.
- Produces:
  - `pub(crate) const MARKER: &str`, `pub(crate) const IN_PROGRESS: &str`
  - `pub(crate) fn is_reserved_name(name: &str) -> bool`
  - `pub(crate) struct Marker { version: u32, leaving: Option<String>, cleared: bool }`
    (Default, Serialize, Deserialize, PartialEq, Clone, Debug)
  - `pub(crate) async fn read_marker(destination: &Path) -> Result<Option<Marker>>`
  - `pub(crate) async fn write_marker(destination: &Path, marker: &Marker) -> Result<()>`
  - `pub(crate) async fn remove_marker(destination: &Path) -> Result<()>`
  - `pub(crate) struct Snapshot { date: NaiveDate, path: PathBuf }`, with
    `fn name(&self) -> String`
  - `pub(crate) fn day_name(NaiveDate) -> String`,
    `pub(crate) fn parse_day(&str) -> Option<NaiveDate>`
  - `pub(crate) async fn list(destination: &Path) -> Result<Vec<Snapshot>>`
    (oldest first)
  - `pub(crate) async fn is_dir(path: &Path) -> bool`
  - `pub(crate) fn clock_today() -> NaiveDate`
  - `pub(crate) fn effective_day(clock: NaiveDate, snapshots: &[Snapshot]) -> NaiveDate`
  - `pub(crate) fn expired(snapshots: &[Snapshot], day: NaiveDate, keep_days: u32) -> Vec<Snapshot>`
  - `pub(crate) fn evictable(snapshots: &[Snapshot]) -> Vec<Snapshot>`

Until Task 7 wires the module in, the library build warns about dead code.
That is expected.

- [ ] **Step 1: Create the module with its tests, bodies left empty**

Create `src-tauri/src/snapshot.rs`:

```rust
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
    todo!()
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
    todo!()
}

pub(crate) async fn write_marker(destination: &Path, marker: &Marker) -> Result<()> {
    todo!()
}

/// Remove the marker; nothing to do when it is already gone.
pub(crate) async fn remove_marker(destination: &Path) -> Result<()> {
    todo!()
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
    todo!()
}

/// A snapshot folder's date, or None for any other name. Strict: exactly
/// `YYYY-MM-DD`, so `2026-9-3` or `2026-09-23 (copy)` is not a snapshot.
pub(crate) fn parse_day(name: &str) -> Option<NaiveDate> {
    todo!()
}

/// The committed snapshots at `destination`, oldest first.
pub(crate) async fn list(destination: &Path) -> Result<Vec<Snapshot>> {
    todo!()
}

pub(crate) async fn is_dir(path: &Path) -> bool {
    todo!()
}

/// Today's local date — or, in a debug build, `DRIVEBY_TODAY` (YYYY-MM-DD),
/// so a run on another day can be tried by hand in `npm run tauri dev`.
pub(crate) fn clock_today() -> NaiveDate {
    todo!()
}

/// The day a run writes: the clock's, or the newest snapshot's when the clock
/// reads earlier. An older-named folder holding the latest state would be
/// taken for history, and retention would delete it first.
pub(crate) fn effective_day(clock: NaiveDate, snapshots: &[Snapshot]) -> NaiveDate {
    todo!()
}

/// What retention deletes: the snapshots dated before `day − keep_days`, but
/// never the newest. A task that has not run for months keeps its last one
/// rather than starting over from nothing.
pub(crate) fn expired(snapshots: &[Snapshot], day: NaiveDate, keep_days: u32) -> Vec<Snapshot> {
    todo!()
}

/// What may go, oldest first, when a destination lacks room: everything but
/// the newest snapshot, which today's is built from or is.
pub(crate) fn evictable(snapshots: &[Snapshot]) -> Vec<Snapshot> {
    todo!()
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
```

Add `mod snapshot;` to `src-tauri/src/main.rs` after `mod scheduler;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: the tests compile and fail, each panicking with "not yet implemented".

- [ ] **Step 3: Implement the bodies**

Replace each `todo!()`:

```rust
pub(crate) fn is_reserved_name(name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case(MARKER) || name.eq_ignore_ascii_case(IN_PROGRESS)
}

pub(crate) async fn read_marker(destination: &Path) -> Result<Option<Marker>> {
    let path = long_path(&destination.join(MARKER));
    match fs::read(&path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).unwrap_or_default())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

pub(crate) async fn write_marker(destination: &Path, marker: &Marker) -> Result<()> {
    persist::write_json_atomic(&destination.join(MARKER), marker).await
}

pub(crate) async fn remove_marker(destination: &Path) -> Result<()> {
    match fs::remove_file(long_path(&destination.join(MARKER))).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

pub(crate) fn day_name(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub(crate) fn parse_day(name: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(name, "%Y-%m-%d").ok()?;
    // chrono accepts `2026-9-3` and a signed year; the round trip does not.
    (day_name(date) == name).then_some(date)
}

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

pub(crate) fn clock_today() -> NaiveDate {
    #[cfg(debug_assertions)]
    if let Some(date) = std::env::var("DRIVEBY_TODAY").ok().as_deref().and_then(parse_day) {
        return date;
    }
    chrono::Local::now().date_naive()
}

pub(crate) fn effective_day(clock: NaiveDate, snapshots: &[Snapshot]) -> NaiveDate {
    snapshots.last().map_or(clock, |newest| clock.max(newest.date))
}

pub(crate) fn expired(snapshots: &[Snapshot], day: NaiveDate, keep_days: u32) -> Vec<Snapshot> {
    let Some(cutoff) = day.checked_sub_days(Days::new(u64::from(keep_days))) else {
        return Vec::new();
    };
    let Some((_newest, older)) = snapshots.split_last() else {
        return Vec::new();
    };
    older.iter().filter(|s| s.date < cutoff).cloned().collect()
}

pub(crate) fn evictable(snapshots: &[Snapshot]) -> Vec<Snapshot> {
    snapshots.split_last().map(|(_, older)| older.to_vec()).unwrap_or_default()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: 8 passed.

- [ ] **Step 5: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass. There will be
dead-code warnings for `snapshot`, which is expected. Commit the files this task touched on the branch.

---

### Task 3: The task option, and the names Driveby keeps

**Files:**
- Modify: `src-tauri/src/backup.rs`:
  - `Task` (lines 36-68) and `impl Task`;
  - `validate_folder_name` (around line 994);
  - `walk_all` (around line 2642);
  - every test `Task { … }` literal.
- Modify: the `Task` literals in the tests of `src-tauri/src/preview.rs` and
  `src-tauri/src/scheduler.rs`.
- Modify: `src/lib/task.js`, `src/lib/__tests__/task.test.js`,
  `src/components/NewTaskForm.jsx:177-183`, `src/lib/i18n.js`.

**Interfaces:**
- Consumes: `crate::snapshot::is_reserved_name` (Task 2).
- Produces:
  - `Task.keep_versions_days: Option<u32>` (JSON `keepVersionsDays`)
  - `Task::keep_versions_days(&self) -> Option<u32>`
  - JS: `export const VERSION_CHOICES = [0, 7, 30, 90, 365]`,
    `export function keepVersionsDays(task): number`, and `folderNameError`
    now also returns `'reserved'`.

- [ ] **Step 1: Add the field so the crate compiles, then write the failing Rust tests**

In `Task`, after the `last_backup` field:

```rust
    /// How many days of daily versions this task keeps at each destination;
    /// absent or 0 is off. See `snapshot.rs`.
    #[serde(default, rename = "keepVersionsDays", skip_serializing_if = "Option::is_none")]
    pub keep_versions_days: Option<u32>,
```

Add the field to every test literal. Each one ends with `last_backup: None,`, and
none of those lines is outside a test module:

```bash
perl -0pi -e 's/^(\s*)last_backup: None,\n/$1last_backup: None,\n$1keep_versions_days: None,\n/mg' \
  src-tauri/src/backup.rs src-tauri/src/preview.rs src-tauri/src/scheduler.rs
grep -c "keep_versions_days: None" src-tauri/src/backup.rs src-tauri/src/preview.rs src-tauri/src/scheduler.rs
```

Expected counts: 17, 2 and 1.

Add these tests to `mod tests` in `backup.rs`:

```rust
    #[test]
    fn versions_are_off_unless_a_positive_number_of_days_is_set() {
        let base = task_with("v", Path::new("/src"), &[]);
        assert_eq!(base.keep_versions_days(), None);
        let with = |days| Task { keep_versions_days: Some(days), ..base.clone() };
        assert_eq!(with(0).keep_versions_days(), None);
        assert_eq!(with(30).keep_versions_days(), Some(30));
        assert_eq!(with(5000).keep_versions_days(), Some(1000), "NTFS allows 1023 links a file");
    }

    #[test]
    fn keep_versions_days_is_read_from_tasks_json() {
        let task: Task =
            serde_json::from_value(serde_json::json!({ "id": "t", "name": "t", "keepVersionsDays": 30 }))
                .unwrap();
        assert_eq!(task.keep_versions_days(), Some(30));
        let older: Task = serde_json::from_value(serde_json::json!({ "id": "t", "name": "t" })).unwrap();
        assert_eq!(older.keep_versions_days(), None);
    }

    #[test]
    fn a_source_folder_cannot_take_a_name_driveby_keeps() {
        assert!(validate_folder_name(".driveby-snapshots").is_err());
        assert!(validate_folder_name(".DriveBy-In-Progress").is_err());
        assert!(validate_folder_name("driveby").is_ok());
    }

    /// A single source is mirrored straight into the destination root, where
    /// these names are Driveby's own. Another destination's marker copied
    /// there would make this one read as daily versions.
    #[tokio::test]
    async fn a_single_source_does_not_bring_driveby_s_names_to_the_root() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join(".driveby-in-progress")).unwrap();
        std::fs::write(source.join(".driveby-in-progress").join("x.txt"), b"x").unwrap();
        std::fs::write(source.join(".driveby-snapshots"), b"{}").unwrap();
        std::fs::write(source.join("kept.txt"), b"kept").unwrap();
        let sources = vec![Source { path: source.to_string_lossy().into(), folder: "source".into() }];

        let walked = walk_all(&sources, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();

        let rels: Vec<&str> = walked.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, ["kept.txt"]);
        assert!(walked.dirs.is_empty());
        assert_eq!(walked.total_bytes, 4);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::`
Expected: compile error, "no method named `keep_versions_days`".

- [ ] **Step 3: Implement**

In `impl Task`, after `destinations()`:

```rust
    /// The days of versions to keep, or None when versions are off. Clamped
    /// to 1000: NTFS allows 1023 links to one file, and a file unchanged for
    /// longer than the retention is linked once for every day kept.
    pub fn keep_versions_days(&self) -> Option<u32> {
        self.keep_versions_days.filter(|&days| days > 0).map(|days| days.min(1000))
    }
```

In `validate_folder_name`, just before the final `Ok(())`:

```rust
    if crate::snapshot::is_reserved_name(name) {
        return Err(anyhow!(
            "\"{}\" is a name Driveby keeps for itself at a destination",
            name
        ));
    }
```

In `walk_all`, replace these lines:

```rust
        merged.total_bytes += walked.total_bytes;
        merged.skipped += walked.skipped;
        merged
            .files
            .extend(walked.files.into_iter().map(|f| FileEntry { rel: join(&f.rel), ..f }));
        merged
            .dirs
            .extend(walked.dirs.into_iter().map(|(p, rel)| (p, join(&rel))));
```

with:

```rust
        // A single source is mirrored straight into the destination root,
        // where these two names are Driveby's own (`snapshot::is_reserved_name`):
        // another destination's marker copied here would make this one read as
        // daily versions.
        let reserved = |rel: &str| {
            !nested && crate::snapshot::is_reserved_name(rel.split('/').next().unwrap_or(rel))
        };
        let (files, dropped): (Vec<FileEntry>, Vec<FileEntry>) =
            walked.files.into_iter().partition(|f| !reserved(&f.rel));
        if !dropped.is_empty() {
            warn!(
                source = %root.display(),
                "left {} file(s) under Driveby's reserved names out of the copy",
                dropped.len()
            );
        }
        let dropped_bytes: u64 = dropped.iter().map(|f| f.size).sum();
        merged.total_bytes += walked.total_bytes - dropped_bytes;
        merged.skipped += walked.skipped;
        merged
            .files
            .extend(files.into_iter().map(|f| FileEntry { rel: join(&f.rel), ..f }));
        merged.dirs.extend(
            walked
                .dirs
                .into_iter()
                .filter(|(_, rel)| !reserved(rel))
                .map(|(p, rel)| (p, join(&rel))),
        );
```

- [ ] **Step 4: Run the Rust tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all pass, the four new tests included.

- [ ] **Step 5: Write the failing JS tests**

In `src/lib/__tests__/task.test.js`, add `keepVersionsDays` and `VERSION_CHOICES`
to the import list from `'../task'`, and append:

```js
describe('keepVersionsDays', () => {
  test('is off unless a positive number of days is set', () => {
    expect(keepVersionsDays({})).toBe(0);
    expect(keepVersionsDays(null)).toBe(0);
    expect(keepVersionsDays({ keepVersionsDays: 0 })).toBe(0);
    expect(keepVersionsDays({ keepVersionsDays: -3 })).toBe(0);
    expect(keepVersionsDays({ keepVersionsDays: 'soon' })).toBe(0);
    expect(keepVersionsDays({ keepVersionsDays: 30 })).toBe(30);
  });

  test('stops at 1000 days, as the backend does', () => {
    expect(keepVersionsDays({ keepVersionsDays: 5000 })).toBe(1000);
  });

  test('offers off and four lengths', () => {
    expect(VERSION_CHOICES).toEqual([0, 7, 30, 90, 365]);
  });
});
```

and inside the existing `describe('folderNameError', …)`:

```js
  test('refuses the names Driveby keeps at a destination', () => {
    expect(folderNameError('.driveby-snapshots')).toBe('reserved');
    expect(folderNameError('.DriveBy-In-Progress')).toBe('reserved');
    expect(folderNameError('driveby')).toBe(null);
  });
```

Run: `npm test -- task`
Expected: FAIL, because `keepVersionsDays` is not exported and `'reserved'`
is never returned.

- [ ] **Step 6: Implement the JS side**

In `src/lib/task.js`, above `folderNameError`:

```js
/// Names Driveby keeps for itself at a destination root — the rule of
/// `snapshot::is_reserved_name` in src-tauri/src/snapshot.rs.
const RESERVED_NAMES = ['.driveby-snapshots', '.driveby-in-progress'];
```

In `folderNameError`, before `return null;`:

```js
  if (RESERVED_NAMES.includes(n.toLowerCase())) return 'reserved';
```

At the end of `src/lib/task.js`:

```js
// ─────────────────────────────────────────────────────────────────────
// Daily versions
// ─────────────────────────────────────────────────────────────────────

/// The lengths the form offers, in days; 0 is off.
export const VERSION_CHOICES = [0, 7, 30, 90, 365];

/// The days of daily versions a task keeps, 0 when versions are off — the
/// rule of `Task::keep_versions_days()` in src-tauri/src/backup.rs: absent,
/// zero, negative or not a number is off, and the ceiling is 1000 (NTFS
/// allows 1023 links to one file).
export function keepVersionsDays(task) {
  const n = Number(task?.keepVersionsDays);
  if (!Number.isFinite(n) || n <= 0) return 0;
  return Math.min(Math.floor(n), 1000);
}
```

In `src/components/NewTaskForm.jsx`, replace:

```js
        const key = folderNameError(misnamed.folder) === 'empty'
          ? 'form.error.source_folder_empty'
          : 'form.error.source_folder_invalid';
```

with:

```js
        const error = folderNameError(misnamed.folder);
        const key = error === 'empty'
          ? 'form.error.source_folder_empty'
          : error === 'reserved'
            ? 'form.error.source_folder_reserved'
            : 'form.error.source_folder_invalid';
```

In `src/lib/i18n.js`, after the `'form.error.source_folder_invalid'` line of
the `en` block:

```js
    'form.error.source_folder_reserved': '“{folder}” is a name Driveby keeps for itself at a destination',
```

and after the same key in the `fr` block:

```js
    'form.error.source_folder_reserved': '« {folder} » est un nom que Driveby se réserve dans une destination',
```

- [ ] **Step 7: Run the JS tests to verify they pass**

Run: `npm test`
Expected: all pass, including the key-parity test in `i18n.test.js`.

- [ ] **Step 8: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` and `npm test` → all pass.
Commit the files this task touched on the branch.

---

### Task 4: snapshot.rs — removing, cloning and moving trees

**Files:**
- Modify: `src-tauri/src/snapshot.rs`
- Modify: `src-tauri/src/backup.rs`: make `entry_names` (around line 965) and
  `is_absent` (around line 981) `pub(crate)`.

**Interfaces:**
- Consumes:
  - `crate::backup::{entry_names, is_absent, CANCELLED_MSG}`;
  - `crate::fsutil::{apply_attrs, blocking, clear_readonly, read_attrs, remove_link_safe}` (Task 1).
- Produces:
  - `pub(crate) async fn remove_tree(root: &Path, token: &CancellationToken) -> Result<()>`
  - `pub(crate) struct CloneStats { linked: u64, copied: u64 }`
    (Debug, Default, PartialEq)
  - `pub(crate) async fn clone_tree(from: &Path, into: &Path, token: &CancellationToken) -> Result<CloneStats>`
  - `pub(crate) struct Moved { moved: u64, blocked: Vec<String> }`
    (Debug, Default, PartialEq)
  - `pub(crate) async fn move_entries(from: &Path, into: &Path, skip: &[&str]) -> Result<Moved>`

- [ ] **Step 1: Write the failing tests**

Change `async fn entry_names` and `async fn is_absent` in `backup.rs` to
`pub(crate) async fn`. Then add to `mod tests` in `snapshot.rs`:

```rust
    use tokio_util::sync::CancellationToken;

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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: compile errors, "cannot find function `remove_tree`", along with
`clone_tree`, `move_entries`, `CloneStats` and `Moved`.

- [ ] **Step 3: Implement**

Extend the `use` lines at the top of `snapshot.rs`:

```rust
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
```

Add after `evictable`:

```rust
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

fn clone_tree_sync(from: &Path, into: &Path, token: &CancellationToken) -> Result<CloneStats> {
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
        let listing = std::fs::read_dir(&dir).with_context(|| format!("list {}", dir.display()))?;
        for entry in listing {
            let entry = entry?;
            let src = entry.path();
            let dst = into_root.join(src.strip_prefix(&from_root).unwrap_or(&src));
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                std::fs::create_dir_all(&dst).with_context(|| format!("create {}", dst.display()))?;
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
pub(crate) async fn move_entries(from: &Path, into: &Path, skip: &[&str]) -> Result<Moved> {
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: all pass. The Windows-only tests are skipped elsewhere.

- [ ] **Step 5: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass. Commit the files this task touched on the branch.

---

### Task 5: snapshot.rs — prepare, leave, commit

**Files:**
- Modify: `src-tauri/src/snapshot.rs`

**Interfaces:**
- Consumes: everything from Tasks 2 and 4.
- Produces:
  - `pub(crate) enum Plan { Mirror { versions_unavailable: bool }, Snapshot { target: PathBuf, day: String, in_progress: bool } }`
    (Debug, Clone, PartialEq)
  - `Plan::target<'a>(&'a self, destination: &'a Path) -> &'a Path`
  - `pub(crate) async fn prepare(destination: &Path, keep_days: u32, clock: NaiveDate, can_link: bool, token: &CancellationToken) -> Result<Plan>`
  - `pub(crate) async fn leave(destination: &Path, token: &CancellationToken) -> Result<()>`
  - `pub(crate) async fn commit(destination: &Path, plan: &Plan) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `snapshot.rs`:

```rust
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
        let marker = Marker { leaving: Some("2026-09-22".into()), cleared: true, ..Marker::default() };
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: compile errors, "cannot find function `prepare`", along with
`leave`, `commit` and `Plan`.

- [ ] **Step 3: Implement**

Add `info` to the tracing import (`use tracing::{info, warn};`). Then add:

```rust
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
            warn!(dest = %destination.display(), "{} is already in the first version; left at the root", name);
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
            "started {} from {}: {} linked, {} copied", name, newest.name(), cloned.linked, cloned.copied
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
    info!(dest = %destination.display(), "daily versions turned off; {} is the backup again", leaving);
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml snapshot::`
Expected: all pass. The Windows-only tests are skipped elsewhere.

- [ ] **Step 5: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass. Commit the files this task touched on the branch.

---

### Task 6: Room with versions — no credit, and eviction

**Files:**
- Modify: `src-tauri/src/backup.rs`:
  - `plan_writes` (around line 1186), `decide_room` (around 1252) and
    `room_check` (around 1274);
  - the call inside `execute_one` (around line 1840);
  - the room tests (around lines 5009-5113).
- Modify: `src-tauri/src/preview.rs:225`, the `plan_writes` call in `plan_one`.

**Interfaces:**
- Consumes: `snapshot::{list, evictable, remove_tree, Snapshot::name}`.
- Produces:
  - `plan_writes(destination, files, nested, parallel, credit_replaced: bool, token)`
  - `decide_room(destination, files, parallel, available, credit_replaced: bool, token)`
  - `room_check(destination, files, parallel, credit_replaced: bool, token)`
  - `pub(crate) async fn fit_by_evicting<F, Fut>(destination: &Path, target: &Path, files: &[FileEntry], parallel: usize, token: &CancellationToken, available: F) -> Result<(u32, Option<Shortfall>)> where F: Fn() -> Fut, Fut: Future<Output = Option<u64>>`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `backup.rs`:

```rust
    /// With daily versions the old copy of a modified file stays, referenced
    /// by the previous day, so the room check credits nothing for it.
    #[tokio::test]
    async fn a_snapshot_is_credited_nothing_for_the_copy_it_replaces() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("grown.bin"), vec![1u8; 9000]).unwrap();
        std::fs::write(dest.join("grown.bin"), vec![1u8; 100]).unwrap();
        let walked = walk(&source, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();

        let plan = plan_writes(&dest, &walked.files, None, 4, false, &CancellationToken::new())
            .await
            .unwrap();

        // All 12 KiB of the new copy, plus the same again while it is in flight.
        assert_eq!(plan.required_bytes, 12288 + 12288);
    }

    #[tokio::test]
    async fn a_destination_short_of_room_gives_up_its_oldest_days_first() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("new.bin"), vec![0u8; 5000]).unwrap();
        for day in ["2026-09-20", "2026-09-21", "2026-09-22"] {
            std::fs::create_dir_all(dest.join(day)).unwrap();
        }
        let target = dest.join(crate::snapshot::IN_PROGRESS);
        std::fs::create_dir_all(&target).unwrap();
        let walked = walk(&source, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();
        let token = CancellationToken::new();

        // Room appears once a single day besides the newest is left.
        let probe = dest.clone();
        let (evicted, short) = fit_by_evicting(&dest, &target, &walked.files, 4, &token, || {
            let probe = probe.clone();
            async move {
                let days = crate::snapshot::list(&probe).await.unwrap().len();
                Some(if days <= 2 { 1 << 30 } else { 100 })
            }
        })
        .await
        .unwrap();
        assert_eq!((evicted, short), (1, None));
        assert!(!dest.join("2026-09-20").exists());
        assert!(dest.join("2026-09-21").exists());

        // Never enough: everything but the newest goes, then the refusal.
        let (evicted, short) =
            fit_by_evicting(&dest, &target, &walked.files, 4, &token, || async { Some(100) })
                .await
                .unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(short, Some(Shortfall { needed: 8192, available: 100 }));
        assert!(dest.join("2026-09-22").exists(), "the newest day is never given up");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::a_snapshot_is_credited`
Expected: compile errors: "this function takes 5 arguments but 6 arguments were
supplied" for `plan_writes`, and "cannot find function `fit_by_evicting`".

- [ ] **Step 3: Implement the flag and `fit_by_evicting`**

In `plan_writes`, add `credit_replaced: bool,` before `token`. In the doc
comment, add after the "a modified file…" bullet:

```rust
/// - with daily versions (`credit_replaced` false), nothing is credited for
///   the copy a modified file replaces: that copy is still referenced by the
///   previous day, so replacing it frees nothing;
```

Replace the `Ok(meta) =>` arm's `let old = …` line with:

```rust
                let old = if meta.is_file() && credit_replaced { on_disk(meta.len()) } else { 0 };
```

Add `credit_replaced: bool,` before `token` in `decide_room` and `room_check`
too, and pass it through:
- in `decide_room`: `plan_writes(destination, files, None, parallel, credit_replaced, token)`;
- in `room_check`: `decide_room(destination, files, parallel, available, credit_replaced, token)`.

In `execute_one`: `room_check(destination, &walked.files, settings.parallel_copies(), true, token)`.

In `preview.rs` `plan_one`:
`plan_writes(destination, &walked.files, nested, parallel, true, token)`.
Task 8 revisits this.

Update the existing test calls, which all mean a mirror:

```bash
perl -pi -e 's/plan_writes\(&dest, &walked\.files, None, 4, &/plan_writes(&dest, &walked.files, None, 4, true, &/; s/decide_room\(&dest, &walked\.files, 4, ([^,]+), &/decide_room(&dest, &walked.files, 4, $1, true, &/' src-tauri/src/backup.rs
grep -n "plan_writes(&dest\|decide_room(&dest" src-tauri/src/backup.rs
```

Expected: 5 lines. The 4 existing calls now have `true,` before the token;
the new test's own call already has `false,`.

Add `use crate::snapshot;` to the `use` block at the top of `backup.rs`, and
add after `room_check`:

```rust
/// With daily versions: give up the oldest days, one at a time, until the run
/// fits — or none is left to give, and the destination is refused for room
/// as before. Returns how many days went, and the shortfall that remains.
///
/// Free space is read again after each deletion rather than predicted: a day
/// frees only the files no other day links to, and nothing short of deleting
/// it says how much that is. `available` is that reading; tests pass their
/// own.
pub(crate) async fn fit_by_evicting<F, Fut>(
    destination: &Path,
    target: &Path,
    files: &[FileEntry],
    parallel: usize,
    token: &CancellationToken,
    available: F,
) -> Result<(u32, Option<Shortfall>)>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<u64>>,
{
    let mut evicted = 0;
    loop {
        // A volume that will not say is not checked, as in `room_check`.
        let Some(free) = available().await else {
            return Ok((evicted, None));
        };
        let Some(short) = decide_room(target, files, parallel, free, false, token).await? else {
            return Ok((evicted, None));
        };
        let days = snapshot::list(destination).await?;
        let Some(oldest) = snapshot::evictable(&days).into_iter().next() else {
            return Ok((evicted, Some(short)));
        };
        warn!(
            dest = %destination.display(),
            "not enough room ({} needed, {} free): deleting the version of {}",
            gigabytes(short.needed),
            gigabytes(short.available),
            oldest.name()
        );
        snapshot::remove_tree(&oldest.path, token).await?;
        evicted += 1;
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all pass, including the existing room tests and the preview tests.

- [ ] **Step 5: Checkpoint**

Commit the files this task touched on the branch.

---

### Task 7: The pipeline writes days

**Files:**
- Modify: `src-tauri/src/backup.rs`:
  - imports;
  - `DestinationOutcome` and `stillborn`;
  - `RunCtx`, `emit_phase` and `maybe_emit`;
  - `copy_one` and `verify_icons_phase`, which call `copy_with_retries`;
  - `copy_with_retries` and `copy_file`;
  - `prune_phase` and `prune_destination`;
  - `execute_all` and `execute_one`;
  - the test helpers `run_one_destination` and `folding_sums_the_work_but_not_the_walk`;
  - the test calls of `prune_destination` and `copy_with_retries`.

**Interfaces:**
- Consumes:
  - `snapshot::{prepare, leave, commit, read_marker, clock_today, Plan}`;
  - `fsutil::{finish_scratch, hard_link_supported, remove_link_safe, replace_link_safe}`;
  - `fit_by_evicting` (Task 6).
- Produces:
  - `DestinationOutcome.snapshot: Option<String>`, `.versions_unavailable: Option<bool>`
    and `.evicted_snapshots: Option<u32>` (JSON `snapshot`,
    `versionsUnavailable`, `evictedSnapshots`);
  - `execute_one(…, settings, clock: NaiveDate, token)`.

- [ ] **Step 1: Outcome fields, and the plumbing that keeps the mirror path identical**

1. In `DestinationOutcome`, after `available_bytes`:

```rust
    /// With daily versions: the day this run wrote, committed or updated in
    /// place. None for a mirror, and for a run stopped before its day was
    /// committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// The task keeps versions, but this drive cannot make hard links, so it
    /// was backed up as a plain mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions_unavailable: Option<bool>,
    /// How many of the oldest days were deleted to make room for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evicted_snapshots: Option<u32>,
```

   In `stillborn`, and in the literal of the test
   `folding_sums_the_work_but_not_the_walk`, add after
   `available_bytes: None,`:

```rust
            snapshot: None,
            versions_unavailable: None,
            evicted_snapshots: None,
```

2. In `RunCtx`, after `target`:

```rust
    /// The destination root. Progress events name it even when `target` is a
    /// day's folder inside it: the root is what the user picked.
    destination: &'a Path,
    /// The run writes a day: replacing and deleting must leave alone the
    /// attributes of files shared with earlier days (`fsutil::replace_link_safe`).
    link_safe: bool,
```

   Replace both occurrences of
   `destination: self.target.to_string_lossy().to_string(),`, in `emit_phase`
   and in `maybe_emit`, with
   `destination: self.destination.to_string_lossy().to_string(),`.

3. `copy_with_retries` and `copy_file` take a new `link_safe: bool` parameter,
   placed just before `on_progress`. `copy_with_retries` passes it on:
   `copy_file(src, dest, token, settings, link_safe, on_progress)`.
   - In `copy_one`:
     `copy_with_retries(&file.path, &dest_path, abort, ctx.settings, ctx.link_safe, &mut |delta| {`.
   - In `verify_icons_phase`:
     `copy_with_retries(&f.path, &dest_path, ctx.token, ctx.settings, ctx.link_safe, &mut |_| {})`.
   - In the test `copy_with_retries_overwrites_readonly_destination`:
     `copy_with_retries(&src, &dest, &token, &settings, false, &mut |_| {})`.

   At the end of `copy_file`, replace:

```rust
    finish_copy(src.to_path_buf(), tmp, dest_l.clone(), mtime).await;
    fs::rename(&tmp_l, &dest_l)
        .await
        .context("commit destination")?;
    Ok(hasher.digest())
```

   with:

```rust
    if link_safe {
        // With daily versions the file being replaced is usually a hard link
        // shared with earlier days, and on Windows its ReadOnly bit is theirs
        // too: leave it, and replace with the rename that does not need it gone.
        finish_scratch(src.to_path_buf(), tmp, mtime).await;
        let (from, to) = (tmp_l.clone(), dest_l.clone());
        blocking(move || replace_link_safe(&from, &to))
            .await
            .context("commit destination")?;
    } else {
        finish_copy(src.to_path_buf(), tmp, dest_l.clone(), mtime).await;
        fs::rename(&tmp_l, &dest_l)
            .await
            .context("commit destination")?;
    }
    Ok(hasher.digest())
```

4. `prune_destination` takes `link_safe: bool` as its last parameter. In
   `prune_phase`:
   `prune_destination(ctx.target, keep, protected, ctx.token, stats, ctx.link_safe)`.
   In the `KeepStatus::Absent` arm, replace:

```rust
                        let outgoing = path.clone();
                        blocking(move || clear_readonly(&outgoing)).await;
                        if fs::remove_file(&path).await.is_ok() {
                            stats.deleted += 1;
                        }
```

   with:

```rust
                        let outgoing = path.clone();
                        let removed = if link_safe {
                            // Today's link only. Clearing +R first would clear
                            // it on every earlier day sharing the file.
                            blocking(move || remove_link_safe(&outgoing)).await.is_ok()
                        } else {
                            blocking(move || clear_readonly(&outgoing)).await;
                            fs::remove_file(&path).await.is_ok()
                        };
                        if removed {
                            stats.deleted += 1;
                        }
```

   Keep the comment above that code. Then add `false` to the seven test calls:

```bash
perl -0pi -e 's/(^[ \t]+prune_destination\(\n(?:[^\n]*\n)*?[ \t]*&mut stats,\n)([ \t]*)\)/$1$2    false,\n$2)/mg' src-tauri/src/backup.rs
grep -n -A7 "        prune_destination($" src-tauri/src/backup.rs | grep -c "false,"
```

   Expected: `7`.

5. Imports at the top of `backup.rs`: extend the `crate::fsutil` list with
   `finish_scratch, hard_link_supported, remove_link_safe, replace_link_safe`,
   and add `use chrono::NaiveDate;` next to `use chrono::Utc;` (or make it
   `use chrono::{NaiveDate, Utc};`).

- [ ] **Step 2: Replace `execute_one` and pass the day from `execute_all`**

In `execute_all`, just before the `for (index, destination)` loop:

```rust
    // One "today" for the whole run, so two destinations written either side
    // of midnight keep the same day.
    let clock = snapshot::clock_today();
```

In the `execute_one(` call, pass `settings, clock, token` instead of
`settings, token`.

Replace the whole of `execute_one` (from `/// Mirror the walked source into one destination.`
to the end of the function) with:

```rust
/// Mirror the walked source into one destination — into its root, or with
/// daily versions into the day's folder (`snapshot.rs`).
#[allow(clippy::too_many_arguments)]
async fn execute_one<R: Runtime>(
    app: &AppHandle<R>,
    backup_id: &str,
    task: &Task,
    destination: &Path,
    dest_index: u32,
    dest_count: u32,
    walked: &mut WalkResult,
    protected: &ProtectedSet<'_>,
    keep: &KeepSet,
    settings: &Settings,
    clock: NaiveDate,
    token: &CancellationToken,
) -> Result<DestinationOutcome> {
    let started = Instant::now();
    let sources = task.sources();

    // Where this run writes. With versions off, a destination that still
    // holds daily versions becomes a mirror again first: a mirror prune would
    // otherwise delete every day as an orphan. A copy 1.7.6 left one level
    // down is put back while the root is still a mirror, so the mirror is
    // whole before anything else happens to it.
    let plan = match task.keep_versions_days() {
        None => {
            snapshot::leave(destination, token).await?;
            put_nested_copy_back(destination, &sources, walked).await;
            snapshot::Plan::Mirror { versions_unavailable: false }
        }
        Some(days) => {
            if snapshot::read_marker(destination).await?.is_none() {
                put_nested_copy_back(destination, &sources, walked).await;
            }
            let probe = destination.to_path_buf();
            let can_link = blocking(move || hard_link_supported(&probe)).await;
            snapshot::prepare(destination, days, clock, can_link, token).await?
        }
    };
    let target = plan.target(destination).to_path_buf();
    let versions = matches!(plan, snapshot::Plan::Snapshot { .. });

    let ctx = RunCtx {
        app,
        backup_id,
        task_id: &task.id,
        target: &target,
        destination,
        link_safe: versions,
        settings,
        token,
        started,
        total_bytes: walked.total_bytes,
        total_files: walked.files.len() as u64,
        dest_index,
        dest_count,
    };
    let mut stats = PhaseStats::default();

    // After the move and the clone, so a backup that is merely one level down,
    // or already in yesterday's version, is not counted as a full copy to make.
    // Before the copy, so a destination that cannot take it is refused with
    // nothing written, instead of filling up hours in and failing with half a
    // copy. Checked per destination, just before its turn: an earlier
    // destination on the same volume has used its share by then. With
    // versions, the oldest days make room first.
    let parallel = settings.parallel_copies();
    let (evicted, short) = if versions {
        let probe = target.clone();
        fit_by_evicting(destination, &target, &walked.files, parallel, token, || {
            let probe = probe.clone();
            async move { blocking(move || available_space(&probe)).await }
        })
        .await?
    } else {
        (0, room_check(destination, &walked.files, parallel, true, token).await?)
    };
    let evicted_snapshots = (evicted > 0).then_some(evicted);
    if let Some(short) = short {
        warn!(
            dest = %destination.display(),
            "not enough space: {} needed, {} free — nothing copied",
            gigabytes(short.needed),
            gigabytes(short.available)
        );
        return Ok(DestinationOutcome {
            evicted_snapshots,
            ..DestinationOutcome::no_space(destination, short)
        });
    }

    let hashes = copy_phase(&ctx, &walked.files, &mut stats).await?;
    #[cfg(any(windows, target_os = "macos"))]
    recase_dirs_phase(&ctx, &walked.dirs, &mut stats).await;
    // A file this destination could not write keeps the copy it already
    // held. Rebuilt only when something failed; a clean run prunes with the
    // set every destination shares.
    let spared;
    let protected = if stats.failed_rels.is_empty() {
        protected
    } else {
        spared = protected.sparing_previous_copies_of(&stats.failed_rels);
        &spared
    };
    prune_phase(&ctx, keep, protected, &mut stats).await?;
    verify_icons_phase(&ctx, &walked.files, &mut stats).await?;
    mirror_dir_attrs_phase(&ctx, &mut walked.dirs, &mut stats).await;

    // Force a final 100% emit so the UI reflects completion even when the
    // throttle would have skipped the last chunk.
    ctx.emit_phase("finishing", ctx.total_bytes, ctx.total_files, Some(0));

    let verified = verify_phase(&ctx, &hashes, &stats).await?;

    for e in &stats.errors {
        warn!("file error: {}", e);
    }

    // A run stopped after its last checkpoint still gets here. It leaves
    // `.driveby-in-progress` for the next run rather than naming a day it did
    // not finish.
    let snapshot_day = match &plan {
        snapshot::Plan::Snapshot { day, .. } if !token.is_cancelled() => {
            snapshot::commit(destination, &plan).await?;
            Some(day.clone())
        }
        _ => None,
    };

    Ok(DestinationOutcome {
        path: destination.to_string_lossy().to_string(),
        status: if stats.failed == 0 {
            DestinationStatus::Success
        } else {
            DestinationStatus::Error
        },
        error: if stats.failed > 0 {
            Some(format!("{} file(s) failed", stats.failed))
        } else {
            None
        },
        total_bytes: Some(ctx.total_bytes),
        total_files: Some(ctx.total_files),
        duration_ms: Some(ctx.started.elapsed().as_millis() as u64),
        skipped: Some(walked.skipped as u64),
        cleaned: Some(stats.deleted),
        unchanged: Some(stats.unchanged),
        failed: Some(stats.failed),
        verified: Some(verified),
        unreadable: Some(walked.unreadable.len() as u64),
        needed_bytes: None,
        available_bytes: None,
        snapshot: snapshot_day,
        versions_unavailable: matches!(plan, snapshot::Plan::Mirror { versions_unavailable: true })
            .then_some(true),
        evicted_snapshots,
    })
}

/// A single source's backup 1.7.6 wrote one level down, moved back up to
/// where the flat layout keeps it — before the copy compares anything, so the
/// run finds its files in place instead of copying all of them again.
async fn put_nested_copy_back(destination: &Path, sources: &[Source], walked: &WalkResult) {
    if let Some(nested) = find_nested_copy(destination, sources, walked).await {
        let moved = move_nested_copy_up(destination, &nested).await;
        info!(
            dest = %destination.display(),
            "moved {} entries back up from the per-source folder {}", moved, nested.folder
        );
    }
}
```

In the tests, replace `run_one_destination` with a pair:

```rust
    async fn run_one_destination<R: Runtime>(
        app: &AppHandle<R>,
        backup_id: &str,
        task: &Task,
        dest: &Path,
        settings: &Settings,
        token: &CancellationToken,
    ) -> Result<DestinationOutcome> {
        let today = crate::snapshot::clock_today();
        run_one_destination_on(app, backup_id, task, dest, settings, token, today).await
    }

    /// `run_one_destination` on a given day, for the daily-versions tests.
    async fn run_one_destination_on<R: Runtime>(
        app: &AppHandle<R>,
        backup_id: &str,
        task: &Task,
        dest: &Path,
        settings: &Settings,
        token: &CancellationToken,
        day: NaiveDate,
    ) -> Result<DestinationOutcome> {
        let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
        let sources = preflight_sources(task)?;
        let folders = destination_folders(&sources);
        let mut walked = walk_all(&sources, &patterns, token).await?;
        let protected = ProtectedSet::new(&walked, &patterns, &folders);
        let keep = KeepSet::new(walked.files.iter().map(|f| f.rel.clone()));
        execute_one(
            app,
            backup_id,
            task,
            dest,
            0,
            1,
            &mut walked,
            &protected,
            &keep,
            settings,
            day,
            token,
        )
        .await
    }
```

- [ ] **Step 3: Run the whole suite, to check the mirror path is unchanged**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: every existing test passes. Nothing about versions is tested yet.

- [ ] **Step 4: Write the daily-version pipeline tests**

Add to `mod tests` in `backup.rs`:

```rust
    fn keeping(task: Task, days: u32) -> Task {
        Task { keep_versions_days: Some(days), ..task }
    }

    async fn run_on_day(
        task: &Task,
        dest: &Path,
        day: &str,
        token: &CancellationToken,
    ) -> Result<DestinationOutcome> {
        let app = tauri::test::mock_app();
        let day = crate::snapshot::parse_day(day).unwrap();
        run_one_destination_on(app.handle(), "versions", task, dest, &Settings::default(), token, day)
            .await
    }

    fn go() -> CancellationToken {
        CancellationToken::new()
    }

    #[tokio::test]
    async fn the_first_run_with_versions_makes_the_mirror_its_first_day_without_copying() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        let task = task_with("v-first", &source, &dests);
        run_on_day(&task, dest, "2026-09-20", &go()).await.unwrap();

        let outcome = run_on_day(&keeping(task, 30), dest, "2026-09-21", &go()).await.unwrap();

        assert_eq!(outcome.snapshot.as_deref(), Some("2026-09-21"));
        assert_eq!(outcome.unchanged, Some(2), "the mirror was moved in, not copied again");
        assert_eq!(std::fs::read(dest.join("2026-09-21/sub/b.txt")).unwrap(), b"beta");
        assert!(!dest.join("a.txt").exists());
        assert!(dest.join(crate::snapshot::MARKER).exists());
    }

    #[tokio::test]
    async fn the_next_day_keeps_yesterday_as_it_was() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        std::fs::write(source.join("same.txt"), b"same").unwrap();
        let task = keeping(task_with("v-next", &source, &dests), 30);
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();

        std::fs::write(source.join("a.txt"), b"alpha, edited").unwrap();
        std::fs::remove_file(source.join("sub/b.txt")).unwrap();
        let outcome = run_on_day(&task, dest, "2026-09-22", &go()).await.unwrap();

        let (day1, day2) = (dest.join("2026-09-21"), dest.join("2026-09-22"));
        assert_eq!(outcome.snapshot.as_deref(), Some("2026-09-22"));
        assert_eq!(std::fs::read(day2.join("a.txt")).unwrap(), b"alpha, edited");
        assert_eq!(std::fs::read(day1.join("a.txt")).unwrap(), b"alpha");
        assert!(!day2.join("sub/b.txt").exists());
        assert_eq!(std::fs::read(day1.join("sub/b.txt")).unwrap(), b"beta");
        assert!(
            crate::fsutil::same_file(&day1.join("same.txt"), &day2.join("same.txt")),
            "an unchanged file costs no room"
        );
        assert!(!dest.join(crate::snapshot::IN_PROGRESS).exists());
    }

    #[tokio::test]
    async fn a_later_run_the_same_day_updates_that_day() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        let task = keeping(task_with("v-same-day", &source, &dests), 30);
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();

        std::fs::write(source.join("a.txt"), b"alpha, again").unwrap();
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();

        assert_eq!(std::fs::read(dest.join("2026-09-21/a.txt")).unwrap(), b"alpha, again");
        assert_eq!(crate::snapshot::list(dest).await.unwrap().len(), 1);
    }

    /// Stopped once the walk is done, as the clone starts — the long part on a
    /// large backup. A token cancelled before the walk would stop the walk
    /// itself and never reach the destination, which is not what this is about.
    #[tokio::test]
    async fn a_stopped_run_names_no_day_and_the_next_run_finishes_it() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        let task = keeping(task_with("v-stop", &source, &dests), 30);
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();
        std::fs::write(source.join("a.txt"), b"alpha, edited").unwrap();

        let app = tauri::test::mock_app();
        let settings = Settings::default();
        let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
        let sources = preflight_sources(&task).unwrap();
        let mut walked = walk_all(&sources, &patterns, &go()).await.unwrap();
        let protected = ProtectedSet::new(&walked, &patterns, &destination_folders(&sources));
        let keep = KeepSet::new(walked.files.iter().map(|f| f.rel.clone()));
        let stopped = go();
        stopped.cancel();
        let day = crate::snapshot::parse_day("2026-09-22").unwrap();
        let result = execute_one(
            app.handle(),
            "versions",
            &task,
            dest,
            0,
            1,
            &mut walked,
            &protected,
            &keep,
            &settings,
            day,
            &stopped,
        )
        .await;
        assert!(result.is_err());
        assert!(!dest.join("2026-09-22").exists(), "a stopped run names no day");
        assert!(dest.join(crate::snapshot::IN_PROGRESS).exists());

        let outcome = run_on_day(&task, dest, "2026-09-23", &go()).await.unwrap();
        assert_eq!(outcome.snapshot.as_deref(), Some("2026-09-23"));
        assert_eq!(std::fs::read(dest.join("2026-09-23/a.txt")).unwrap(), b"alpha, edited");
        assert!(!dest.join(crate::snapshot::IN_PROGRESS).exists());
    }

    /// A source whose drive is unplugged today is carried into today's
    /// version from yesterday's, as the mirror leaves its folder alone.
    #[tokio::test]
    async fn a_source_missing_today_keeps_yesterdays_copy_in_todays_version() {
        let root = tempfile::tempdir().unwrap();
        let (here, gone, dest) =
            (root.path().join("here"), root.path().join("gone"), root.path().join("dest"));
        for (dir, file) in [(&here, "h.txt"), (&gone, "g.txt")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(file), b"x").unwrap();
        }
        std::fs::create_dir_all(&dest).unwrap();
        let task = Task {
            source: None,
            sources: Some(vec![
                Source { path: here.to_string_lossy().into(), folder: "Here".into() },
                Source { path: gone.to_string_lossy().into(), folder: "Gone".into() },
            ]),
            ..keeping(task_with("v-missing", &here, &[dest.clone()]), 30)
        };
        run_on_day(&task, &dest, "2026-09-21", &go()).await.unwrap();

        std::fs::remove_dir_all(&gone).unwrap();
        run_on_day(&task, &dest, "2026-09-22", &go()).await.unwrap();

        assert_eq!(std::fs::read(dest.join("2026-09-22/Gone/g.txt")).unwrap(), b"x");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_file_that_cannot_be_read_today_keeps_yesterdays_version() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        std::fs::write(source.join("vm.bin"), b"yesterday").unwrap();
        let task = keeping(task_with("v-locked", &source, &dests), 30);
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();

        std::fs::write(source.join("vm.bin"), b"today, and locked").unwrap();
        let _locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(source.join("vm.bin"))
            .unwrap();
        let outcome = run_on_day(&task, dest, "2026-09-22", &go()).await.unwrap();

        assert_eq!(outcome.failed, Some(1), "the staged failure has to have happened");
        assert_eq!(std::fs::read(dest.join("2026-09-22/vm.bin")).unwrap(), b"yesterday");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_readonly_file_keeps_its_bit_in_yesterdays_version() {
        use crate::fsutil::read_attrs;
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        for name in ["edited.txt", "deleted.txt"] {
            std::fs::write(source.join(name), b"v1").unwrap();
            apply_attrs(&source.join(name), 0x1);
        }
        let task = keeping(task_with("v-readonly", &source, &dests), 30);
        run_on_day(&task, dest, "2026-09-21", &go()).await.unwrap();

        clear_readonly(&source.join("edited.txt"));
        std::fs::write(source.join("edited.txt"), b"version two").unwrap();
        apply_attrs(&source.join("edited.txt"), 0x1);
        clear_readonly(&source.join("deleted.txt"));
        std::fs::remove_file(source.join("deleted.txt")).unwrap();
        let outcome = run_on_day(&task, dest, "2026-09-22", &go()).await.unwrap();

        let day1 = dest.join("2026-09-21");
        assert_eq!(outcome.failed, Some(0));
        assert_eq!(std::fs::read(dest.join("2026-09-22/edited.txt")).unwrap(), b"version two");
        assert_eq!(std::fs::read(day1.join("edited.txt")).unwrap(), b"v1");
        assert!(!dest.join("2026-09-22/deleted.txt").exists());
        for name in ["edited.txt", "deleted.txt"] {
            assert_ne!(read_attrs(&day1.join(name)).unwrap() & 0x1, 0, "{name} lost its +R");
        }
        for path in [
            day1.join("edited.txt"),
            day1.join("deleted.txt"),
            dest.join("2026-09-22/edited.txt"),
            source.join("edited.txt"),
        ] {
            clear_readonly(&path);
        }
    }

    #[tokio::test]
    async fn turning_versions_off_makes_the_newest_day_the_backup_again() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let dest = &dests[0];
        let task = task_with("v-off", &source, &dests);
        run_on_day(&keeping(task.clone(), 30), dest, "2026-09-21", &go()).await.unwrap();
        std::fs::write(source.join("a.txt"), b"alpha, edited").unwrap();
        run_on_day(&keeping(task.clone(), 30), dest, "2026-09-22", &go()).await.unwrap();

        let outcome = run_on_day(&task, dest, "2026-09-23", &go()).await.unwrap();

        assert_eq!(outcome.snapshot, None);
        assert_eq!(outcome.unchanged, Some(2), "the newest day came up by renaming");
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"alpha, edited");
        let mut names: Vec<String> = std::fs::read_dir(dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sub"]);
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::`
Expected: all pass. Run each failing test on its own, and fix the pipeline
wiring rather than the assertion: these tests are the spec's guarantees.

- [ ] **Step 6: Checkpoint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass, and the build
no longer warns about dead code in `snapshot`. (`preview_base` and
`restorable_days` come in Task 8.)
Run: `awk 'length > 100' src-tauri/src/backup.rs src-tauri/src/snapshot.rs` →
review any new line it reports and wrap it by hand. Commit the files this task touched on the branch.

---

### Task 8: Preview, restore, and the list of days

**Files:**
- Modify: `src-tauri/src/snapshot.rs` (`preview_base`, `DayInfo`,
  `restorable_days`, and tests).
- Modify: `src-tauri/src/preview.rs`: `DestinationPreview`, `plan()` and
  `plan_one()`, and a test.
- Modify: `src-tauri/src/restore.rs`: `restore()` and a test.
- Modify: `src-tauri/src/main.rs`: the `list_snapshots` command and its
  registration.

**Interfaces:**
- Consumes: Tasks 2 and 5.
- Produces:
  - `pub(crate) async fn preview_base(destination: &Path, versions: bool, clock: NaiveDate) -> Result<PathBuf>`
  - `#[derive(Serialize)] pub(crate) struct DayInfo { name: String, path: String }`
  - `pub(crate) async fn restorable_days(destination: &Path) -> Result<Vec<DayInfo>>`
    (newest first)
  - `DestinationPreview.versions: bool` (JSON `versions`)
  - Tauri command `list_snapshots(destination: String) -> Vec<DayInfo>`
    (JS: `invoke('list_snapshots', { destination })`)

- [ ] **Step 1: Write the failing tests**

In `snapshot.rs` tests:

```rust
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
```

In `restore.rs` tests:

```rust
    #[tokio::test]
    async fn the_root_of_a_backup_with_daily_versions_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(backup.join("2026-09-21")).unwrap();
        std::fs::write(backup.join("2026-09-21/a.txt"), b"a").unwrap();
        std::fs::write(backup.join(crate::snapshot::MARKER), b"{}").unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let app = tauri::test::mock_app();

        let err = restore(app.handle(), &CancellationToken::new(), backup.clone(), dest.clone())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("choose a day"), "{err}");

        restore(app.handle(), &CancellationToken::new(), backup.join("2026-09-21"), dest.clone())
            .await
            .unwrap();
        assert!(dest.join("a.txt").exists(), "a day restores like any backup");
    }
```

In `preview.rs` tests:

```rust
    /// With daily versions the run starts from the newest day, so that is
    /// what the preview compares against. A file gone from the source is not
    /// in today's version, but it is not deleted.
    #[tokio::test]
    async fn a_preview_with_versions_counts_against_the_newest_day() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        let day = dest.join("2026-09-21");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(dest.join(crate::snapshot::MARKER), b"{}").unwrap();
        std::fs::write(source.join("same.txt"), b"identical").unwrap();
        std::fs::copy(source.join("same.txt"), day.join("same.txt")).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(
            &std::fs::metadata(source.join("same.txt")).unwrap(),
        );
        filetime::set_file_mtime(day.join("same.txt"), mtime).unwrap();
        std::fs::write(source.join("new.txt"), b"brand new").unwrap();
        std::fs::write(day.join("old.txt"), b"gone from source").unwrap();
        let task = Task { keep_versions_days: Some(30), ..task_for(&source, &[&dest]) };

        let payload = plan(&task, &Settings::default(), &CancellationToken::new()).await.unwrap();

        let d = &payload.destinations[0];
        assert!(d.versions);
        assert_eq!(d.path, dest.to_string_lossy());
        assert_eq!((d.unchanged_files, d.new_files, d.deleted_files), (1, 1, 1));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: compile errors, "cannot find function `restorable_days`" and
`preview_base`, and "no field `versions`".

- [ ] **Step 3: Implement**

In `snapshot.rs`:

```rust
/// The tree a run would start from, for the preview. The preview writes
/// nothing, so it cannot move or clone anything; it looks where the run would
/// find the files instead: today's day, else the newest one (today's is
/// cloned from it), else the snapshot a turning-off is bringing up, else
/// `.driveby-in-progress`. A mirror, or a destination whose versions are
/// about to begin, is its own root.
pub(crate) async fn preview_base(
    destination: &Path,
    versions: bool,
    clock: NaiveDate,
) -> Result<PathBuf> {
    let Some(marker) = read_marker(destination).await? else {
        return Ok(destination.to_path_buf());
    };
    let snapshots = list(destination).await?;
    if versions {
        let today = destination.join(day_name(effective_day(clock, &snapshots)));
        if is_dir(&today).await {
            return Ok(today);
        }
    }
    if let Some(leaving) = marker.leaving {
        return Ok(destination.join(leaving));
    }
    let in_progress = destination.join(IN_PROGRESS);
    Ok(match snapshots.last() {
        Some(newest) => newest.path.clone(),
        None if is_dir(&in_progress).await => in_progress,
        None => destination.to_path_buf(),
    })
}

/// One day a destination can be restored from.
#[derive(Serialize, Debug, PartialEq)]
pub(crate) struct DayInfo {
    pub(crate) name: String,
    pub(crate) path: String,
}

/// The days a destination with daily versions can be restored from, newest
/// first; none for a mirror, whose root is the backup. The path is built
/// here so the frontend never joins paths.
pub(crate) async fn restorable_days(destination: &Path) -> Result<Vec<DayInfo>> {
    if read_marker(destination).await?.is_none() {
        return Ok(Vec::new());
    }
    Ok(list(destination)
        .await?
        .into_iter()
        .rev()
        .map(|s| DayInfo { name: s.name(), path: s.path.to_string_lossy().to_string() })
        .collect())
}
```

In `restore.rs` `restore()`, right after `reject_overlap(&backup_path, &destination)?;`:

```rust
    // A backup with daily versions holds one whole backup per day; restoring
    // its root would pour every day into one folder. The UI asks which day.
    if crate::snapshot::read_marker(&backup_path).await?.is_some() {
        return Err(anyhow!("This backup keeps daily versions; choose a day to restore"));
    }
```

In `preview.rs`:
- Add `use crate::snapshot;`.
- Add to `DestinationPreview`:

```rust
    /// The task keeps daily versions: what the run leaves out of today's
    /// version stays in the earlier days, so it is not a deletion to warn
    /// about.
    pub versions: bool,
```

- `plan_one` gains `credit_replaced: bool` before `token` and passes it to
  `plan_writes`.
- In `plan()`, add `let clock = snapshot::clock_today();` before the loop. In
  the loop, replace everything from `// The run moves a copy 1.7.6 left one
  level down` to the end of the `previews.push(plan_one(…).await?);` statement
  with:

```rust
        // The tree the run would start from: with daily versions, today's day
        // or the newest one, which today's is cloned from. The preview moves
        // and clones nothing, so it looks there instead.
        let versions = task.keep_versions_days().is_some();
        let base = snapshot::preview_base(destination, versions, clock).await?;
        // The run moves a copy 1.7.6 left one level down back up before it
        // compares anything. The preview does not move it — a dry run that
        // renames is not dry — but counts as if it had, or it would show
        // that whole copy as deleted and every file as new. Only a mirror
        // root can hold one.
        let nested = if base == *destination {
            find_nested_copy(destination, &sources, &walked).await
        } else {
            None
        };
        let mut preview = plan_one(
            &base,
            &walked,
            &keep,
            &protected,
            nested.as_ref(),
            settings.parallel_copies(),
            !versions,
            token,
        )
        .await?;
        preview.path = destination.to_string_lossy().to_string();
        preview.versions = versions;
        previews.push(preview);
```

In `main.rs`, after `cancel_restore`:

```rust
/// The days a destination with daily versions can be restored from, newest
/// first — empty for a plain mirror, whose root is the backup.
#[tauri::command]
async fn list_snapshots(destination: String) -> Result<Vec<snapshot::DayInfo>, String> {
    snapshot::restorable_days(std::path::Path::new(&destination))
        .await
        .map_err(|e| e.to_string())
}
```

Add `list_snapshots,` to `generate_handler!` after `cancel_restore,`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all pass, and there are no dead-code warnings left from `snapshot`.

- [ ] **Step 5: Checkpoint**

Commit the files this task touched on the branch.

---

### Task 9: The frontend

**Files:**
- Modify: `src/lib/tauri.js`, `src/lib/format.js`,
  `src/lib/__tests__/format.test.js`
- Modify: `src/components/ConfirmDialog.jsx`, `src/components/NewTaskForm.jsx`,
  `src/components/Home.jsx`
- Modify: `src/context/AppContext.jsx`: `restoreBackup`, and the success
  notification in the `onBackupComplete` listener.
- Modify: `src/components/History.jsx`, `src/components/BackupPreviewDialog.jsx`
- Modify: `src/lib/i18n.js`

**Interfaces:**
- Consumes:
  - the `list_snapshots` command;
  - outcome fields `snapshot`, `versionsUnavailable` and `evictedSnapshots`;
  - `DestinationPreview.versions`;
  - `keepVersionsDays` and `VERSION_CHOICES` (Task 3).
- Produces:
  - `bridge.listSnapshots(destination) -> Promise<[{ name, path }]>`;
  - `formatDay(dayKey)` in `makeFormatters`;
  - `confirm({ …, choices, initialChoice })`, which resolves with the picked
    value;
  - `restoreBackup(backupPath, preferredDay)`.

- [ ] **Step 1: Write the failing formatter test**

In `src/lib/__tests__/format.test.js`, inside `describe('makeFormatters (fr)', …)`:

```js
  test('a day of daily versions reads with its weekday', () => {
    expect(fr.formatDay('2026-09-23')).toBe('mer. 23/09/2026');
    expect(makeFormatters('en').formatDay('2026-09-23')).toBe('Wed 23/09/2026');
    expect(fr.formatDay('not a day')).toBe('not a day');
  });
```

Run: `npm test -- format`
Expected: FAIL, "fr.formatDay is not a function".

- [ ] **Step 2: Implement `formatDay`**

In `src/lib/format.js`, after `formatTime`:

```js
  // A day of daily versions ("YYYY-MM-DD", a folder name) as the restore
  // dialog lists it: the weekday first, since "the version from three days
  // ago" is how the day is remembered, then the date as formatTime writes one.
  const formatDay = (dayKey) => {
    const [y, m, d] = String(dayKey).split('-').map(Number);
    if (!y || !m || !d) return String(dayKey);
    return `${weekdayShort.format(new Date(y, m - 1, d))} ${pad(d)}/${pad(m)}/${y}`;
  };
```

Add `formatDay,` to the returned object. Run `npm test -- format` → PASS.

- [ ] **Step 3: Add the strings**

In `src/lib/i18n.js`, `en` block. After `'form.hint.schedule'`:

```js
    'form.label.versions': 'Daily versions',
    'form.hint.versions': 'One version per day; later runs the same day update it. Needs an NTFS, APFS or ext4 drive.',
    'form.versions.off': 'Off',
    'form.versions.days.one': 'Keep {n} day',
    'form.versions.days.other': 'Keep {n} days',
    'form.versions.year': 'Keep 1 year',
    'form.versions.off_confirm.title': 'Turn off daily versions?',
    'form.versions.off_confirm.body': 'At the next backup, the latest version becomes the backup and the earlier versions are deleted.',
    'form.versions.off_confirm.action': 'Turn off',
    'form.versions.fewer_confirm.title': 'Keep fewer versions?',
    'form.versions.fewer_confirm.body.one': 'Versions older than {n} day will be deleted at the next backup.',
    'form.versions.fewer_confirm.body.other': 'Versions older than {n} days will be deleted at the next backup.',
    'form.versions.fewer_confirm.action': 'Keep fewer',
```

After `'backup.notification.body'`:

```js
    'backup.notification.body_no_versions': '“{name}” is backed up, but {path} cannot keep daily versions.',
```

After `'preview.label.unchanged'`:

```js
    'preview.label.deleted_versions': 'not in today’s version',
```

After `'restore.dialog.action'`:

```js
    'restore.day.title': 'Restore which day?',
    'restore.day.body': 'This backup keeps one version per day. Choose the day to restore.',
    'restore.day.action': 'Choose',
```

After `'history.unreadable.other'`:

```js
    'history.versions_unavailable': 'No versions kept on {path}: this drive cannot make hard links (exFAT or FAT32?)',
    'history.evicted.one': 'The oldest version on {path} was deleted to make room',
    'history.evicted.other': 'The {n} oldest versions on {path} were deleted to make room',
```

In the `fr` block, at the same anchors:

```js
    'form.label.versions': 'Versions quotidiennes',
    'form.hint.versions': 'Une version par jour ; les exécutions suivantes du même jour la mettent à jour. Nécessite un disque NTFS, APFS ou ext4.',
    'form.versions.off': 'Désactivées',
    'form.versions.days.one': 'Garder {n} jour',
    'form.versions.days.other': 'Garder {n} jours',
    'form.versions.year': 'Garder 1 an',
    'form.versions.off_confirm.title': 'Désactiver les versions quotidiennes ?',
    'form.versions.off_confirm.body': 'À la prochaine sauvegarde, la dernière version devient la sauvegarde et les versions précédentes sont supprimées.',
    'form.versions.off_confirm.action': 'Désactiver',
    'form.versions.fewer_confirm.title': 'Garder moins de versions ?',
    'form.versions.fewer_confirm.body.one': 'Les versions de plus de {n} jour seront supprimées à la prochaine sauvegarde.',
    'form.versions.fewer_confirm.body.other': 'Les versions de plus de {n} jours seront supprimées à la prochaine sauvegarde.',
    'form.versions.fewer_confirm.action': 'Réduire',
```

```js
    'backup.notification.body_no_versions': '« {name} » est sauvegardé, mais {path} ne peut pas garder de versions quotidiennes.',
```

```js
    'preview.label.deleted_versions': 'absents de la version du jour',
```

```js
    'restore.day.title': 'Restaurer quel jour ?',
    'restore.day.body': 'Cette sauvegarde garde une version par jour. Choisissez le jour à restaurer.',
    'restore.day.action': 'Choisir',
```

```js
    'history.versions_unavailable': 'Aucune version gardée sur {path} : ce disque ne peut pas créer de liens physiques (exFAT ou FAT32 ?)',
    'history.evicted.one': 'La plus ancienne version sur {path} a été supprimée pour faire de la place',
    'history.evicted.other': 'Les {n} plus anciennes versions sur {path} ont été supprimées pour faire de la place',
```

Run: `npm test` → PASS. `i18n.test.js` checks that both locales have the same
keys.

- [ ] **Step 4: Add `listSnapshots` to the bridge, and choices to `ConfirmDialog`**

In `src/lib/tauri.js`, after `cancelRestore`:

```js
  // [{ name: 'YYYY-MM-DD', path }], newest first; [] for a plain mirror.
  listSnapshots: (destination) => invoke('list_snapshots', { destination }),
```

Replace `src/components/ConfirmDialog.jsx` with:

```jsx
import React, { useEffect, useRef, useState } from 'react';
import Button from './common/Button';
import { useExitTransition } from '../hooks/useExitTransition';
import { useT } from '../hooks/useT';

/// `state.choices` ([{ value, label }]) turns the dialog into a question with
/// a list: it resolves with the value picked instead of `true`, and with
/// `false` when dismissed, as always. `state.initialChoice` is preselected.
export default function ConfirmDialog({ state, onResolve }) {
  const t = useT();
  const dialogRef = useRef(null);
  const lastStateRef = useRef(state);
  if (state) lastStateRef.current = state;
  const [choice, setChoice] = useState(null);
  // Read by the keyboard handler, which is registered once per dialog and
  // would otherwise see the choice as it was when the dialog opened.
  const choiceRef = useRef(null);
  choiceRef.current = choice;

  const { mounted, state: anim } = useExitTransition(!!state, 200);

  useEffect(() => {
    if (state?.choices) setChoice(state.initialChoice ?? state.choices[0]?.value ?? null);
  }, [state]);

  useEffect(() => {
    if (!state) return;
    dialogRef.current?.focus();
    // Capture phase, so this runs before the app-wide shortcuts that also
    // listen on window. Two bubble-phase listeners on the same target can't
    // suppress each other, which is why cancelling a confirmation used to
    // close the task form behind it and throw away what was typed (#F5).
    const openedAt = Date.now();
    const handler = (e) => {
      if (e.key !== 'Escape' && e.key !== 'Enter') return;
      // Holding or double-tapping the key that opened this dialog fires a
      // second keydown ~500ms later. Confirming on that repeat destroyed
      // things before the dialog had been read (#F1).
      if (e.repeat) return;
      if (e.key === 'Escape') {
        e.stopPropagation();
        onResolve(false);
        return;
      }
      if (Date.now() - openedAt < 250) return;
      if (!dialogRef.current?.contains(document.activeElement)) return;
      e.stopPropagation();
      onResolve(state.choices ? choiceRef.current : true);
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [state, onResolve]);

  if (!mounted) return null;
  const visible = state || lastStateRef.current;
  if (!visible) return null;

  return (
    <div
      className="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-title"
      data-state={anim}
    >
      <div className="modal__backdrop" onClick={() => onResolve(false)} />
      <div className="modal__content" ref={dialogRef} tabIndex={-1}>
        <h2 id="confirm-title" className="modal__title">{visible.title}</h2>
        <p className="modal__body">{visible.body}</p>
        {visible.choices && (
          <select
            className="field"
            value={choice ?? ''}
            onChange={(e) => setChoice(e.target.value)}
            aria-label={visible.title}
            style={{ width: '100%', marginTop: 12 }}
          >
            {visible.choices.map((c) => (
              <option key={c.value} value={c.value}>{c.label}</option>
            ))}
          </select>
        )}
        <div className="modal__actions">
          <Button onClick={() => onResolve(false)}>{t('common.cancel')}</Button>
          <Button
            variant="primary"
            destructive={visible.danger}
            onClick={() => onResolve(visible.choices ? choice : true)}
          >
            {visible.confirmLabel || t('common.ok')}
          </Button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Restore asks which day; notification warns when versions are unavailable**

In `src/context/AppContext.jsx`, replace the beginning of `restoreBackup` up to
and including the `bridge.restoreBackup` call:

```js
  const restoreBackup = useCallback(async (backupPath) => {
    if (restoreBusy.current) {
      showToast(tr('restore.busy'), 'error');
      return;
    }
    const destination = await bridge.selectDirectory(tr('restore.dialog.select'));
    if (!destination) return;
    const ok = await confirm({
      title: tr('restore.dialog.title'),
      // Naming only the destination made it impossible to notice that a
      // shifted History row had selected a different backup (#F7).
      body: tr('restore.dialog.body', { source: backupPath, destination }),
      confirmLabel: tr('restore.dialog.action'),
    });
    if (!ok) return;
    restoreBusy.current = true;
    beginRestore();
    try {
      const res = await bridge.restoreBackup(backupPath, destination);
```

with:

```js
  const restoreBackup = useCallback(async (backupPath, preferredDay = null) => {
    if (restoreBusy.current) {
      showToast(tr('restore.busy'), 'error');
      return;
    }
    // A destination with daily versions holds one whole backup per day, and
    // the backend refuses to restore its root. Ask which day, preselecting the
    // one this History row wrote while it is still kept.
    let source = backupPath;
    const days = await bridge.listSnapshots(backupPath).catch(() => []);
    if (days.length > 0) {
      const { formatDay } = makeFormatters(currentLanguage());
      const picked = await confirm({
        title: tr('restore.day.title'),
        body: tr('restore.day.body'),
        choices: days.map((d) => ({ value: d.path, label: formatDay(d.name) })),
        initialChoice: (days.find((d) => d.name === preferredDay) || days[0]).path,
        confirmLabel: tr('restore.day.action'),
      });
      if (!picked) return;
      source = picked;
    }
    const destination = await bridge.selectDirectory(tr('restore.dialog.select'));
    if (!destination) return;
    const ok = await confirm({
      title: tr('restore.dialog.title'),
      // Naming only the destination made it impossible to notice that a
      // shifted History row had selected a different backup (#F7).
      body: tr('restore.dialog.body', { source, destination }),
      confirmLabel: tr('restore.dialog.action'),
    });
    if (!ok) return;
    restoreBusy.current = true;
    beginRestore();
    try {
      const res = await bridge.restoreBackup(source, destination);
```

In the `onBackupComplete` listener, replace:

```js
          if (settingsRef.current.showNotifications) {
            bridge.notify(
              tr('backup.notification.title'),
              tr('backup.notification.body', { name: existingTask?.name || tr('view.tasks').toLowerCase() }),
```

with:

```js
          if (settingsRef.current.showNotifications) {
            const name = existingTask?.name || tr('view.tasks').toLowerCase();
            // The task keeps daily versions, and this drive cannot. The backup
            // is fine, but the user is counting on something that is not there,
            // and a scheduled run has no other way to say so.
            const noVersions = (data.destinations || []).find((d) => d.versionsUnavailable);
            bridge.notify(
              tr('backup.notification.title'),
              noVersions
                ? tr('backup.notification.body_no_versions', { name, path: noVersions.path })
                : tr('backup.notification.body', { name }),
```

- [ ] **Step 6: History and the preview dialog**

In `src/components/History.jsx`:
- `onClick={() => restoreBackup(dest.path)}` → `onClick={() => restoreBackup(dest.path, dest.snapshot)}`
- `onClick={() => restoreBackup(only.path)}` → `onClick={() => restoreBackup(only.path, only.snapshot)}`
- After the `{entry.unreadable > 0 && ( … )}` block:

```jsx
                  {destinations.filter((d) => d.versionsUnavailable).map((d, i) => (
                    <div key={`nv-${i}`} className="history-path" style={{ color: 'var(--system-orange)' }}>
                      {t('history.versions_unavailable', { path: d.path })}
                    </div>
                  ))}
                  {destinations.filter((d) => d.evictedSnapshots > 0).map((d, i) => (
                    <div key={`ev-${i}`} className="history-path" style={{ color: 'var(--system-orange)' }}>
                      {t('history.evicted', { n: d.evictedSnapshots, count: d.evictedSnapshots, path: d.path })}
                    </div>
                  ))}
```

In `src/components/BackupPreviewDialog.jsx`:
- `deletions: acc.deletions + d.deletedFiles,` → `deletions: acc.deletions + (d.versions ? 0 : d.deletedFiles),`
- Replace:

```jsx
                    <span className={`preview-stat ${d.deletedFiles > 0 ? 'preview-stat--danger' : ''}`}>
                      <b>{formatNumber(d.deletedFiles)}</b> {t('preview.label.deleted')}
```

  with:

```jsx
                    {/* With daily versions, what today's version leaves out
                        stays in the earlier days: counted, not a warning. */}
                    <span className={`preview-stat ${d.deletedFiles > 0 && !d.versions ? 'preview-stat--danger' : ''}`}>
                      <b>{formatNumber(d.deletedFiles)}</b> {t(d.versions ? 'preview.label.deleted_versions' : 'preview.label.deleted')}
```

- [ ] **Step 7: The form**

In `src/components/Home.jsx`:
- Add `confirm,` to the `useApp()` destructuring.
- Add `confirm={confirm}` after `showToast={showToast}` in `<NewTaskForm …/>`.

In `src/components/NewTaskForm.jsx`:
- Add `keepVersionsDays,` and `VERSION_CHOICES,` to the `'../lib/task'` import.
- Add `keepVersionsDays: 0,` to `INITIAL`.
- In the `initialTask` branch of the `useState` initialiser, add
  `keepVersionsDays: keepVersionsDays(initialTask),`.
- Add `confirm` to the props:
  `({ onAdd, onSave, onCancel, defaultDestination, showToast, confirm, initialTask, dataState, otherTasks = [] })`.
- `const submit = () => {` → `const submit = async () => {`.
- In `submit`, just before
  `// Folder names are stored trimmed, the way both sides read them, so`:

```js
    // Fewer days, or none, deletes versions at the next run. Asked here,
    // while it is still a choice: the run itself asks nobody.
    const before = keepVersionsDays(initialTask);
    const after = task.keepVersionsDays;
    if (isEdit && before > 0 && after < before && confirm) {
      const ok = await confirm(after === 0
        ? {
            title: t('form.versions.off_confirm.title'),
            body: t('form.versions.off_confirm.body'),
            confirmLabel: t('form.versions.off_confirm.action'),
            danger: true,
          }
        : {
            title: t('form.versions.fewer_confirm.title'),
            body: t('form.versions.fewer_confirm.body', { n: after, count: after }),
            confirmLabel: t('form.versions.fewer_confirm.action'),
            danger: true,
          });
      if (!ok) return;
    }
```

- After the `{task.schedule === 'custom' && ( … )}` block, before `<div className="card__actions">`:

```jsx
      <FormField label={t('form.label.versions')} hint={t('form.hint.versions')}>
        <select
          className="field"
          value={task.keepVersionsDays}
          onChange={(e) => setTask({ ...task, keepVersionsDays: Number(e.target.value) })}
        >
          {VERSION_CHOICES.map((days) => (
            <option key={days} value={days}>
              {days === 0
                ? t('form.versions.off')
                : days === 365
                  ? t('form.versions.year')
                  : t('form.versions.days', { n: days, count: days })}
            </option>
          ))}
        </select>
      </FormField>
```

- [ ] **Step 8: Run the frontend checks**

Run: `npm test` → all pass.
Run: `npm run build` → builds with no error.

- [ ] **Step 9: Checkpoint**

Commit the files this task touched on the branch.

---

### Task 10: README and CHANGELOG for 2.0.0

**Files:**
- Modify: `README.md`: the "What it does" section and "Limits".
- Modify: `CHANGELOG.md`: a new `## 2.0.0` section at the top.

- [ ] **Step 1: README**

Replace the paragraph that starts with `**A mirror, not snapshots.**` with:

```markdown
**A mirror, or daily versions.** By default the destination reflects the
source as it is now: files you delete at the source are removed from the copy
on the next run. Turn on *Daily versions* for a task and each destination
keeps one complete folder per day instead — `2026-09-23`, `2026-09-22`, … —
for 7, 30 or 90 days, or a year. Files that did not change are hard links to
the day before, so a day costs only what changed. To get one file back, open
that day's folder; to get a whole day back, restore it from History. Versions
need a drive that can make hard links (NTFS, APFS, ext4); on exFAT or FAT32 the
task backs up as a mirror and says so. They also rely on *Preserve file
modification time*, which is on by default.
```

In the `**Restore.**` paragraph, after "choose where to put it", add: ", after
choosing the day when the task keeps daily versions".

In `## Limits`, replace the `**No versioned snapshots.**` bullet with:

```markdown
- **Daily versions need hard links.** exFAT and FAT32 drives — the usual
  format of USB sticks — are backed up as a mirror. On a drive with versions,
  a folder's Properties in Explorer counts every day at its full size; the
  drive's free space is the real figure.
- **Going back to Driveby 1.x deletes daily versions.** 1.x does not know the
  layout and removes every day's folder as something the source does not have.
  Turn versions off and run the task once before installing an older version.
```

- [ ] **Step 2: CHANGELOG**

Insert under `# Changelog`:

```markdown
## 2.0.0

**Daily versions.** A task can now keep one version of its backup per day,
for 7, 30 or 90 days or a year: choose it under *Daily versions* when you
create or edit the task. Each destination then holds a folder per day —
`2026-09-23`, `2026-09-22`, … — and each one is a complete backup you can open
and browse. A file deleted or damaged by accident can be taken back as it was
on any of those days: open that day's folder, or restore the whole day from
History, which now asks which day.

A day costs only what changed. Files that did not change since the day before
are hard links to the same copy, not copies, so thirty days of a folder where
little changes take little more room than one. Several runs on the same day
update that day's version. When the drive runs out of room, the oldest days
are deleted first, and History says how many; the most recent day is never
deleted.

Turning versions on moves the backup already on the drive into the first day.
That is a rename, so it is instant and needs no room. Turning them off, or
keeping fewer days, asks first, then deletes the versions it no longer keeps
at the next run; the latest day becomes the backup again.

Versions need a drive that can make hard links: NTFS on Windows, APFS on
macOS, ext4 and most Linux filesystems. On exFAT or FAT32 — the usual format
of USB sticks — the task goes on backing up as before, as a mirror, and History
and the notification say that no versions were kept.

**Before going back to 1.x.** Driveby 1.x does not know about daily versions:
on a drive that holds them, it would delete every day as a folder the source
does not have. Turn versions off and run the task once before installing an
older version.
```

- [ ] **Step 3: Checkpoint**

Read both files once in full, and check that no other paragraph still says
Driveby has no versions (`grep -n -i "snapshot\|version" README.md`).
Commit the files this task touched on the branch.

---

## Final verification (after Task 10)

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`: all pass. The test
  count goes up from the start of this plan. The platform-gated tests differ
  by OS, which is expected.
- [ ] `npm test` and `npm run build`: pass.
- [ ] `awk 'length > 100' src-tauri/src/snapshot.rs`: nothing unexpected.
- [ ] By hand on Windows, `npm run tauri dev`:
  1. Edit a task and set Daily versions to 30 days, then run it. The
     destination shows `.driveby-snapshots` and today's folder, and the run is
     as fast as an ordinary incremental run.
  2. Change one file and delete another at the source. Set
     `DRIVEBY_TODAY` to tomorrow in the shell that runs `npm run tauri dev`,
     restart the app, and run the task again. Both days are in Explorer:
     yesterday still has both files as they were.
  3. History → Restore. The day picker lists both days, with that row's day
     preselected. Restore yesterday into an empty folder.
  4. Edit the task and set versions to Off. Saving asks first. Run it: the
     latest day is back at the root, and no date folders or marker remain.
- [ ] On the Kali VM (ext4), the same steps 1-2.
- [ ] If an exFAT USB stick is at hand: a task with versions writes a plain
  mirror, and History shows the orange "No versions kept" line.
- [ ] Leave the branch unpushed and unmerged, and hand over to the user for their own
  check. The release (bump to 2.0.0, CI, tag) is theirs to start.
