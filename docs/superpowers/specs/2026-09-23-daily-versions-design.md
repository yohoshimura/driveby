# Daily versions — design

**Status:** approved 2026-09-23. Ships in Driveby 2.0.0.

## Problem

A task is a mirror. Each run makes the destination match the source, so a
file deleted or damaged at the source is deleted or damaged in the backup at
the next run. The README names this as a deliberate limit: "No versioned
snapshots … versioning changes the storage model entirely."

The ask: optionally keep one version per day, so that a file deleted or
corrupted by accident can be taken back as it was three days ago, without
using much extra space.

Today an older copy is lost in exactly two places, both in `backup.rs`:

- `copy_file` — the closing `fs::rename(tmp, dest)` replaces the old copy of a
  modified file;
- `prune_destination` — `fs::remove_file` in the `KeepStatus::Absent` arm
  deletes the copy of a file the source no longer has.

## Decisions

1. **Whole snapshots, one folder per day.** `<dest>/2026-09-23/` holds a
   complete tree, and every file unchanged since the day before is a hard link
   to that day's copy. The chosen alternative to moving old copies aside into a
   versions folder: every day can be browsed and restored as a whole backup.
2. **A destination that cannot hold hard links** (exFAT, FAT32, some network
   shares) is still backed up, as a plain mirror, and the run says that no
   versions were kept there. An optional feature never costs a backup.
3. **A destination without room** gives up its oldest snapshots, one at a
   time, until the run fits (the Time Machine rule). The most recent snapshot
   is never given up. A run that still does not fit is refused for room as it
   is today.
4. **Turning versions off** is confirmed in the form. The next run moves the
   most recent snapshot back up to the destination root, where it becomes the
   mirror again, and deletes the other snapshots.
5. **Per task**, in the task form next to the schedule: Off, 7 days, 30 days,
   90 days, 1 year. Turning it on picks 30 days. It is per task because the
   layout of a destination is decided per task.

## Layout

```
D:\Backup\                         (a destination of a task with versions on)
├─ .driveby-snapshots              marker: {"version":1,"leaving":null,"cleared":false}
├─ 2026-09-21\                     a committed snapshot: the whole tree
├─ 2026-09-23\                     the most recent one
└─ .driveby-in-progress\           the snapshot being built; resumable
```

**The marker decides the layout, never folder names.** A source can hold a
folder called `2026-09-21`, and a single source is mirrored straight into the
destination root, so a name that looks like a date proves nothing. Without the
marker the root is a mirror. With it, the root holds snapshots. The marker is
JSON, written atomically (scratch file and rename), and read by one function.
Its `leaving` and `cleared` fields are only set while versions are being
turned off. A marker that cannot be parsed still marks the layout, and one
that cannot be read fails the destination: reading it as "no marker" would
let a mirror prune delete every snapshot.

A **snapshot** is a directory at the root whose name parses as a local date,
`YYYY-MM-DD`. Snapshots are ordered by that date. Inside a snapshot the
layout is exactly what the mirror's is today: one source's contents directly,
or one folder per source.

In the snapshot layout, anything at the root that is not the marker, a
snapshot or `.driveby-in-progress` is left alone, whatever it is. That
includes the destination root's own `desktop.ini`, which
`is_root_icon_marker` already protects.

**One snapshot per day.** A second run on the same day updates that day's
snapshot in place. "Today" is the local date when the run starts. If the
clock reads a day earlier than the newest snapshot, the run writes into the
newest snapshot rather than creating an older-named one, because an
older-named folder would then be treated as history while it held the latest
state.

## A run on a destination with versions on

`execute_one` gains a preparation step before the room check and a commit
step after the verify phase. Everything between them — copy, re-case, prune,
icon verification, directory attributes, verify — runs unchanged, with
`ctx.target` set to the snapshot being written instead of the destination
root. The steps, per destination:

1. **Hard-link probe.** Create a small file at the destination root, hard-link
   it, remove both.
   - It fails and **there is no marker**: the destination is backed up as a
     mirror, exactly as with versions off. Its outcome carries
     `versionsUnavailable: true`.
   - It fails and **there is a marker**: the destination fails with an error
     and nothing is touched. Falling back to a mirror there would prune every
     snapshot as an orphan.
2. **Moving an existing mirror into the first snapshot** (no marker yet).
   First `find_nested_copy` / `move_nested_copy_up` run as they do today, so
   the mirror is in its own layout. Then every root entry except
   `.driveby-in-progress` and the root `desktop.ini` is **renamed** into
   `.driveby-in-progress/`, and the marker is written **last**. Renames only:
   `is_absent` is asked again right before each one, and nothing is deleted.
   The backup already there becomes the first day without a byte being
   copied.

   The marker goes last so that an interrupted move is simply redone: with no
   marker, the next run finds `.driveby-in-progress/` and the remaining root
   entries and carries on. With the marker first, an interruption would leave
   mirror entries at a root that the snapshot layout then never touches.
3. **Retention.** Delete every snapshot dated before `today − N days`, except
   the most recent one. A task that has not run for two months keeps its last
   snapshot, rather than starting over from nothing. A `.driveby-in-progress/`
   found beside an existing snapshot for today is a leftover, and is deleted
   too.
4. **The target.**
   - If today's snapshot exists, write into it.
   - Otherwise write into `.driveby-in-progress/`, creating it if needed, and
     first **clone the most recent snapshot into it with hard links**. This is
     rsnapshot's `cp -al` followed by rsync.

   The clone:
   - creates every directory and hard-links every file;
   - skips what already exists, so an interrupted clone or an interrupted run
     resumes where it stopped;
   - can be cancelled;
   - leaves out symlinks and `*.driveby-tmp` scratch files;
   - copies a file instead when linking it fails, for example at NTFS's limit
     of 1023 links per file.

   A resumed `.driveby-in-progress/` is cloned into the same way, whichever
   day it was started on: whatever it already holds is kept, and the sync
   corrects it.
5. **Room.** `room_check` runs against the target. In snapshot mode
   `plan_writes` credits nothing for a replaced file: its old copy is still
   referenced by the previous snapshot, so the replacement frees nothing.

   If the room is short, delete the oldest snapshot other than the most recent
   one and other than the target, read the free space again, and decide again.
   When there is nothing left to delete, the destination is refused for room
   as it is today. The number deleted is reported as `evictedSnapshots`.
6. **The pipeline, unchanged, on the target.** Because the target starts as a
   clone of the previous day, the guarantees the mirror already has carry over
   without new code:
   - an excluded or unreadable path keeps the copy the clone brought;
   - a file whose new copy fails keeps the previous day's version: the
     scratch file is never renamed over the cloned link, and prune leaves the
     link alone because the source still has the file;
   - a replaced file is written to a scratch file and renamed over the link,
     which replaces today's directory entry and leaves the previous day's file
     exactly as it was;
   - prune removes today's link to a deleted file and nothing else.
7. **Commit.** Rename `.driveby-in-progress/` to today's name. Only when the
   pipeline returned Ok and the run was not cancelled. A run with some failed
   files commits: those files hold the previous day's version. A cancelled or
   aborted run leaves `.driveby-in-progress/` for the next run to resume.
   A day on which no run commits simply has no snapshot.

`find_nested_copy` only runs while there is no marker. Inside snapshots the
tree is Driveby's own.

## Hard links and the read-only bit

On Windows, the Hidden, System and ReadOnly attributes belong to the **file**,
not to the link. The pipeline changes one attribute of a file that already
exists at the destination, in two places:

- `finish_copy` calls `clear_readonly(&dest)` so the rename can replace a
  `+R` file;
- prune calls `clear_readonly(&outgoing)` so a `+R` file can be deleted.

On a linked file, both would strip `+R` from the same file in every older
snapshot, and so would deleting an expired snapshot. So `fsutil.rs` gains two
calls that act without touching the attribute. Both were measured on NTFS with
rustc 1.98 before this was written:

- `replace_link_safe(tmp, dest)` — `SetFileInformationByHandle` with
  `FileRenameInfoEx` and the flags
  - `FILE_RENAME_FLAG_REPLACE_IF_EXISTS` (0x1),
  - `FILE_RENAME_FLAG_POSIX_SEMANTICS` (0x2),
  - `FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE` (0x40).

  Measured: it replaces a `+R` link atomically. The other link keeps its
  content and its `+R`. Without 0x40 the call is refused, and so is
  `std::fs::rename`. windows-sys 0.59 has `FileRenameInfoEx` and
  `FILE_RENAME_INFO`, whose union carries `Flags`, but not the rename flag
  constants under the features in use. They are declared locally, with
  `winbase.h` cited. When the call fails as unsupported (an older Windows, a
  filesystem without the class), it falls back to clearing the bit and
  renaming, with a warning in the log.
- `remove_link_safe(path)` — `std::fs::remove_file`, with **no**
  `clear_readonly` before it. Measured: std already deletes a `+R` file on
  NTFS through `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`, and the
  other link keeps its `+R`. What strips the bit is the `clear_readonly` prune
  makes first. Only on a filesystem without that disposition class, where
  std answers PermissionDenied, does it fall back to clearing and deleting.

On Unix both are a plain `rename` or `remove_file`: unlinking depends on the
directory's permissions, not the file's.

They are used **only when the run writes a snapshot**. `RunCtx` carries a
flag, and `copy_file` and prune choose by it. On that path `copy_file` stamps
the scratch file with `finish_scratch`, which is `finish_copy` without the
`clear_readonly`. The mirror path stays byte-for-byte what it is today.

Directories are never linked, since each snapshot has its own. So the
directory attribute passes (`mirror_dir_attrs_phase`, prune's `+R` handling
for directories) are unaffected.

Deleting a snapshot (retention, eviction, turning versions off) walks it
depth-first:
- files are removed with `remove_link_safe`;
- symlinks are removed as links and never followed;
- directories get their own `+R` cleared, then are removed.

## Turning versions off

Versions off, and a marker at the root:

1. If `leaving` is unset, set it to the most recent snapshot, or to
   `.driveby-in-progress` when there is no committed one, and write the
   marker.
2. Unless the marker says `cleared`, delete every other snapshot, and
   `.driveby-in-progress` unless it is the one leaving. Then set `cleared`
   and write the marker. From that point on, a folder at the root named like
   a date may be a source folder that already came up, so a resumed run must
   not list the root for snapshots to delete again.
3. Rename each top-level entry of the one leaving up to the root, asking
   `is_absent` right before each rename. An entry whose name is already taken
   at the root fails the destination, naming the entry. The same happens for
   any rename that fails. Nothing is copied or pruned, the marker stays, and
   the next run tries again.
4. Remove the emptied folder, then the marker.
5. The run continues as an ordinary mirror run.

Every step can be redone after an interruption. `leaving` names the snapshot
coming back up, and `cleared` says the deleting is over. Together they mean a
source folder named like a date that has already moved to the root is never
taken for a snapshot.

Versions turned on again while a marker still says `leaving` finish turning
them off first, since the user confirmed deleting those versions. The
destination is then a mirror, and becomes the first snapshot again.

Versions off, **no marker**, and a `.driveby-in-progress/` at the root means a
first move into snapshots was interrupted and versions were then turned off.
Its entries are moved back up by the same code, with `leaving` set to it.

A destination removed from a task keeps whatever it holds. Driveby no longer
writes to it.

## Preview, restore, history

**Preview** (`preview.rs`) compares the source with the tree the run will
start from:
- today's snapshot if it exists;
- else the most recent snapshot;
- else `.driveby-in-progress/`;
- else the root, when versions have just been turned on and the root is
  still a mirror.

It makes no probe and writes nothing. In snapshot mode, "to delete" reads as
"not in today's version, kept in earlier ones", and the room figure uses the
no-credit rule. It does not predict evictions.

**Restore** (`restore.rs`) refuses a backup path that holds a marker: "This
backup keeps daily versions; choose a day." A new command,
`list_snapshots(destination)`, returns `[{ name, path }]` for the committed
snapshots, newest first. The path is built in Rust, so the frontend never
joins paths.

**History → Restore** calls `list_snapshots` first:
- **non-empty:** a dialog asks which day. It lists the days, newest first,
  formatted in the reader's language, and preselects the day that History row
  wrote if it is still there. Then come the folder picker and the confirmation
  as today, restoring from that day's `path`.
- **empty:** the flow is unchanged.

To get back **one file**, Reveal opens the destination root, where each day is
an ordinary folder.

**`DestinationOutcome`** gains three fields, all `skip_serializing_if` None:
- `snapshot`: the day this run wrote, committed or updated;
- `versionsUnavailable`: the probe failed, and the destination was mirrored;
- `evictedSnapshots`: how many snapshots were deleted for room.

History shows the last two in orange, the way `unreadable` is shown today. The
notification of a run with `versionsUnavailable` says that the drive cannot
keep versions. Nothing is folded to the run level: the frontend reads the
destinations, which the history row already stores whole.

## Data model

```rust
// Task
#[serde(default, rename = "keepVersionsDays", skip_serializing_if = "Option::is_none")]
pub keep_versions_days: Option<u32>,
```

Absent or 0 means off. `Task::keep_versions_days()` returns the number clamped
to 1..=1000, or None. `src/lib/task.js` reads it by the same rule. An older
Driveby ignores the field and would run a mirror over a snapshot destination —
see *Behaviour changes*.

`.driveby-snapshots` and `.driveby-in-progress` become reserved names:
- `validate_folder_name` refuses them as a source's folder;
- in a single-source mirror, the walk leaves a top-level entry with either
  name out of the copy, with a warning in the log. Copying another
  destination's marker to this root would make it read as snapshots.

The form (`NewTaskForm.jsx`):
- a Versions select after the schedule, with a hint: "One version per day;
  later runs the same day update it. Needs an NTFS, APFS or ext4 drive."
- saving a task whose value went down asks first: "Versions older than N days
  will be deleted at the next backup."
- saving a task whose value went to Off asks first: "Earlier versions will be
  deleted at the next backup; the latest becomes the backup."

Strings in English and French.

A debug-only environment variable, `DRIVEBY_TODAY=YYYY-MM-DD`
(`cfg(debug_assertions)`), overrides "today", so a run on another day can be
tried by hand in `npm run tauri dev`.

## Behaviour changes worth naming

- **A downgrade to Driveby 1.x** treats a snapshot
  destination as a mirror. Its prune would delete every snapshot folder as an
  orphan, since none of them is in the source. The changelog and the release
  notes say so. There is no way to protect a destination from a version that
  cannot read the marker.
- Explorer's folder Properties counts each day at its full size, because it
  counts every link. The README says so, with the destination drive's free
  space as the real figure.
- The first run after turning versions on is as fast as an ordinary run: the
  existing mirror is renamed, not copied.
- Progress events keep naming the destination root, not the snapshot folder
  being written, so the progress line reads as it does today.
- With `preserveMtime` off, every run re-copies every file. That is true
  today, and with versions on each day then costs the full size. The README
  says versions rely on it.

## Testing

Rust, in the end-to-end style of `backup.rs`'s tests (`mock_app()`, scratch
directories), with "today" passed in rather than read from the clock:

- The first run moves an existing mirror into today's snapshot without copying
  anything (`copied_files == 0`), and writes the marker last.
- The next day, an unchanged file is the same file in both snapshots. A test
  helper compares inode and device on Unix, and volume serial and file index
  (`GetFileInformationByHandle`) on Windows. A modified file is new today and
  intact yesterday. A deleted file is absent today and present yesterday.
- A second run on the same day updates the snapshot in place.
- Retention deletes expired snapshots and keeps the most recent even when it
  has expired.
- Eviction, through a `decide_room`-style function that takes the free space
  as a parameter: oldest first, never the most recent, then refusal.
- A cancelled run leaves `.driveby-in-progress/`. The next run resumes it and
  commits it.
- Probe failure, with the probe's answer injected: without a marker, a mirror
  and `versionsUnavailable`; with a marker, an error and nothing pruned.
- Turning off: the latest comes up and the others are deleted. The same again
  after an interruption at each step.
- An interrupted first move is finished by the next run.
- A clock earlier than the newest snapshot writes into the newest snapshot.
- An unreadable source subfolder and a file whose copy fails both keep the
  previous day's version in today's snapshot.
- Windows only: a `+R` file in yesterday's snapshot is still `+R` after today
  replaces it, after today prunes it, and after an expired snapshot linking
  it is deleted.
- `restore` refuses a snapshot root. `list_snapshots` lists committed days
  only, newest first.
- The reserved names: refused as a folder, left out of a single-source walk.

Frontend (`npm test`): `keepVersionsDays` normalisation in `task.js`.

By hand, `npm run tauri dev` on Windows (NTFS):
1. Turn versions on for a task, run it.
2. Change and delete a file, set `DRIVEBY_TODAY` to the next day, run again.
3. Browse both days in Explorer, restore a day, turn versions off.

Then the same on the Kali VM (ext4). If possible, a run to an exFAT USB stick,
for the mirror fallback and its warning.

## Out of scope

- A browser inside the app for the files of each day. Explorer, Finder and
  Files already browse the days.
- More than one version per day.
- Versions on a drive without hard links.
- Predicting evictions in the preview.
- A different setting per destination of the same task.
- Reporting how much room the versions take.
- APFS clones (`clonefile`) instead of hard links on macOS.
