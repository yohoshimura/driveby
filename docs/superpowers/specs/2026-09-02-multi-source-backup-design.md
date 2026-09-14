# Multi-source backup — design

**Status:** approved 2026-09-02. Extends the plural `destinations` model
introduced in 1.7.2.

## Problem

A task backs up exactly one source folder. Someone who wants several folders
on one drive has to make one task per folder — and each of those tasks
mirror-prunes the destination against its own source, so they cannot share a
destination at all. `reject_foreign_overlaps` refuses that configuration
outright, correctly: the alternative is two tasks deleting each other's files
and reporting it as a clean-up.

The ask is one task, several sources, one destination.

## The layout decision

`copy_one` writes `ctx.target.join(&file.rel)`, where `rel` is relative to the
source root. **The destination is a mirror of the source's contents**, not
`destination/<source name>/…`. With several sources that sentence has no single
meaning, so the layout had to be settled before anything else.

**Decision: one subfolder per source, always — including for a single source.**

```
C:\Photos\      →   D:\Backup\
C:\Documents\        ├─ Photos\
                     │   ├─ 2024\
                     │   └─ cat.jpg
                     └─ Documents\
                         └─ tax.pdf
```

The layout never depends on how many sources a task has, so nothing in the
code is conditional on the count and there is nothing to explain to the user.

**Accepted cost:** every backup written by 1.7.4 or earlier changes shape. On
the first run after the upgrade the copy phase writes every file into
`dest/<Folder>/`, and prune — which runs *after* it (`execute_one`: copy,
recase-dirs, prune, verify-icons, mirror-attrs) — then finds the old root-level
files absent from the keep set, which now says `Photos/…`, and deletes them.

So the order is copy-then-delete, not delete-then-copy: **there is no window
during which the destination holds no backup.** What it does cost is room for
both copies at once, and moving every byte a second time. An earlier draft of
this document had the order backwards and warned of a gap that cannot occur;
the phases are the authority.

**A failed copy keeps its old copy.** `continueOnError` defaults to true, so a
run where some files fail to copy still reaches prune with `stats.failed > 0`.
On the reshape run, a file whose copy fails would be in neither place the
destination is checked against: the new `dest/<Folder>/a.txt` was never
written, and the old flat `dest/a.txt` is absent from the keep set — which now
says `<Folder>/a.txt` — so prune would delete it. Before this change the same
failure was harmless, because the failed file's destination path *was* the
keep entry.

An earlier version of this document accepted that as one unlucky file, on the
grounds that the old flat rel is not derivable from the new one. Both halves
were wrong. The likely failure on that run is a full disk, because the run
needs room for both copies, and then it is *every* file that did not fit — the
whole remainder of the backup, which prune then deleted. And the old rel is
derivable: it is the new rel with its folder taken off.

So prune spares it. `copy_phase` records the rels it failed, and the
destination that failed them prunes with
`ProtectedSet::sparing_previous_copies_of`, which also covers each one's
unprefixed path. The spared copy stays only while writing the new one keeps
failing: a destination without room for two copies converges over a few runs
— each prunes what the last one managed to move — and nothing already backed
up goes missing in between. Once the layout has moved, nothing normally lives at
those paths: in steady state a failed copy spares a path that holds nothing,
or at worst keeps an orphan one run longer.

This is not a heuristic in the delete path: it can only keep more, never
delete more, and it keeps nothing past the run that writes the new copy.

One overlap it cannot cover. A source holding a subfolder named like its own
destination folder — `C:\Photos\Photos\` — has old and new layouts sharing
paths, and the copy phase itself overwrites the old copy of the nested file
with the new copy of its namesake one level up. If the nested file's own copy
then fails, it has none until a run succeeds. That needs the name collision,
the overwrite and the failure together, and the source still holds the file.

Chosen deliberately over a rename-based migration. No migration code is
written.

## The pipeline

### Approach: prefix `rel` at merge time, keep one `WalkResult`

`FileEntry` carries an absolute `path` (where to read from) and a `rel` (where
to write to). `rel` is the *only* thing that decides the destination path. So:

1. Walk each source separately, with `walk()` **exactly as it is today**.
2. Rewrite every `rel` in the result to `<folder>/<rel>`.
3. Concatenate into one `WalkResult`.

Everything downstream is untouched: one `WalkResult`, one `KeepSet`, one
`ProtectedSet`, one prune over the whole destination, one progress total, one
`DestinationOutcome` per destination. `CompletePayload` does not change shape.

All the multi-source logic lives in one new function sitting above `walk`:

```rust
/// Walk every source and merge the results into the single WalkResult the
/// rest of the pipeline already expects, each entry's `rel` prefixed with the
/// destination subfolder its source writes into.
async fn walk_all(
    sources: &[Source],
    patterns: &glob::PatternSet,
    token: &CancellationToken,
) -> Result<WalkResult>
```

- **Prefixes:** `files[].rel`, `dirs[].1`, and every member of `excluded` and
  `unreadable`.
- **Sums:** `total_bytes`, `skipped`.
- **Adds:** one `(source_root_path, folder)` entry to `dirs` per source, so a
  source with no files still materialises its folder in
  `mirror_dir_attrs_phase`.

`walk()` itself stays single-source and knows nothing about subfolders.

**The preview pipeline gets the same treatment.** `preview.rs::plan` is not
downstream of `execute_all` — it is a parallel pipeline that calls
`preflight_source` and `walk` itself, then builds its own `ProtectedSet` and
`KeepSet`. It switches to `walk_all` and to `Task::sources()` alongside the
backup pipeline, or the preview would report on a layout the run does not
produce, which is worse than no preview at all.

### Rejected: sources looped inside destinations

`for destination { for source { execute_one(source, destination/folder) } }`
would keep `execute_one` almost unchanged and isolate per-source failure
naturally. Rejected because prune would never see above a subfolder:
**removing a source from a task would leave its subfolder at the destination
forever**, with nothing able to clean it up. That is the opposite of what a
mirror backup is for.

## Data model

`Task.source: String` becomes a legacy/plural pair, in the exact shape
`destination`/`destinations` took in 1.7.2, and for the same reason: the
scheduler deserialises this struct and can tick before the frontend migration
has run, and a user who downgrades writes the old shape back.

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Source {
    pub path: String,
    /// The destination subfolder this source writes into. A folder name, not
    /// a path: no separators, no `.` or `..`. Empty until named, for a source
    /// with no name of its own.
    pub folder: String,
}

pub struct Task {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,          // written by 1.7.4 and earlier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<Source>>,
    // … destination / destinations / schedule / … unchanged
}
```

`Task::sources()` mirrors `Task::destinations()`: `sources` wins when present
and non-empty, otherwise the legacy `source` folds to a single entry whose
`folder` is that path's own basename. Blank paths dropped, exact repeats
collapsed; filesystem questions are left to the guards that have to run
anyway.

**A source with no name of its own keeps an empty folder.** A drive root —
`D:\`, a share root, `/` — has no basename, and no name is invented for it.
It is kept rather than dropped: dropping it made the frontend migration
rewrite tasks.json without the path, and every run after reported that no
source was set, a whole-drive backup quietly unconfigured by an update.
`preflight_sources` refuses it by its path instead, and the form shows the
empty field to fill. The same holds for any listed source whose folder is
blank: dropping one of several backed up the rest and reported success while
that one was never read.

"No basename" is `Path::file_name()`, which parses by the rules of the OS it
is built for: `D:` is a drive prefix on Windows and an ordinary folder name on
Linux and macOS. The frontend cannot ask `Path`, so it recognises the roots a
folder dialog can hand back on each platform — a bare drive, a share root,
`/` — and agrees with Rust on every path a dialog produces.

Frontend `migrateTasks` gains the matching rewrite and must stay idempotent —
it already returns the same array when there is nothing to do, and the
existing test for `destinations` has exactly that shape.

## What is refused, and where

Every rule is enforced twice: in the form at the moment of picking, so the
message can name the offending folder, and in the backend, which must refuse
independently because the form is not the only way a task reaches disk.

Two existing signatures take a single source and become plural:
`preflight_source(task)` and `reject_destination_overlaps(source, destinations)`.
The latter keeps its per-pair logic and gains an outer loop over sources, plus
the source-versus-source check below.

| Rule | Mechanism |
|---|---|
| Source ⊄ destination, both directions | `reject_overlap` — exists, now once per source |
| Destinations not nested in one another | `reject_destination_overlaps` — exists |
| Nothing another task already uses | `reject_foreign_overlaps` — extended to all sources |
| **Sources not nested in one another** | **new** — otherwise the inner one is backed up twice, into two subfolders |
| **Subfolder names unique within a task** | **new** — compared through `fold_rel`, so `Photos` and `photos` collide where the filesystem folds them |
| **Every source has a subfolder name** | **new** — `preflight_sources`, quoting the path, since an empty name gives the message nothing else to quote |

A subfolder name is validated as a folder name, not a path: non-empty, no `/`
or `\`, not `.` or `..`, no characters the platform rejects.

Duplicate names are **refused, never auto-disambiguated**. Appending `(2)`
would make the on-disk layout depend on list order, so reordering the sources
would silently relocate terabytes.

## Safety: a missing source must not delete its backup

The failure this design must not have: a source on an unplugged drive, prune
finding its subfolder absent from the keep set, and deleting the only copy of
it that exists.

`ProtectedSet::covers` already protects an entry **and its whole subtree** —
`prune_destination` skips a covered entry without descending into it. So:

- A source whose `walk()` returns `Err` (root missing, unreadable, or not a
  directory) contributes **no files**, and instead inserts its subfolder `rel`
  into the merged `unreadable`. Prune walks around it. The run continues on the
  other sources and reports the failure.
- When **every** source fails, `walk_all` returns `Err`. Not a `""` entry in
  `unreadable`: that would stop prune correctly but still let the run report
  success, stamp `lastBackup`, and leave the scheduler believing the task ran.
  Today a single missing source fails the run outright, and that must not
  regress. An `Err` here preserves it — `execute_one` never runs, so
  `source_root_unreadable()` needs no multi-source generalisation at all.

A source that fails while others succeed makes the run **incomplete, not
failed**: its count flows into `walked.unreadable`, which `CompletePayload`
already surfaces as "this backup is knowingly incomplete but nothing was
deleted for it".

## Behaviour changes worth naming

1. **Existing backups are reshaped** on the first run after the upgrade — every
   byte re-copied one level down, then the old flat mirror pruned. It needs
   room for both copies at once; without it, the reshape takes several runs and
   keeps every old copy it has not yet replaced. Accepted; see *The layout
   decision*. It goes in the CHANGELOG.
2. **A task whose only source is a drive root stops until it is named.** The
   run refuses it, quoting the path, and the form shows the empty folder name.
   It goes in the CHANGELOG too, since it is the one upgrade that asks
   something of the user.
3. **A source-root `desktop.ini` becomes the subfolder's icon descriptor.**
   This does **not** fall out of prefixing, as an earlier draft claimed. `walk`
   carried an explicit suppression — a source-root `desktop.ini` was dropped
   and recorded as excluded — and it is keyed on the *unprefixed* source rel,
   so `walk_all` cannot reach it. Removing that block is what relocates the
   descriptor, and it is safe because the effect that mattered survives
   elsewhere: `ProtectedSet::covers` tests `is_root_icon_marker(rel)` before it
   consults `excluded` at all, so an existing destination-root `desktop.ini` is
   still protected from prune. The suppression existed only because a source
   root used to map onto the destination root. It no longer does.

## Interface

The Source field becomes a list modelled on the existing `dest-list`, reusing
`field-row`, with one editable field added per row:

```
Sources
┌───────────────────────────┬────────┬─────────────┬────────┐
│ C:\Users\y\Photos         │ Choose │ Photos      │ Remove │
│ C:\Work\Archives\Photos   │ Choose │ Photos-Work │ Remove │
└───────────────────────────┴────────┴─────────────┴────────┘
  readonly, title=full path            editable,
                                       filled on pick
                        [ + Add a source ]
```

The subfolder is pre-filled from the folder's basename when the source is
picked, and stays editable. A root has no basename, so its field starts empty
and marked. On a name collision the row shows the error and the user resolves
it.

- `src/lib/task.js`: add `taskSources(task)`, a subfolder-name validator, and
  extend `findForeignOverlap` to consider every source.
- `TaskCard.jsx:14` builds `${task.source} → …` and must account for N sources.
- New keys in **both** the `en` and `fr` blocks of `src/lib/i18n.js`.

**No new warning UI is built.** The preview already tells the truth:
`confirmBeforeBackup` defaults to `true` and gates the preview dialog, and
`DestinationPreview` already carries `deletedFiles` / `deletedBytes`. The first
preview of an existing task after the upgrade reads "40 000 deleted / 40 000
new" with real byte counts — the warning, in the app's own vocabulary, with
real numbers. A bespoke "this looks like a reshape" banner would be one more
heuristic that can be wrong.

## Testing

**A cost to name:** always-a-subfolder inserts one path component into ~59
existing assertions (`backup.rs` 52, `preview.rs` 7; `restore.rs`'s 16 are
restore-side and unaffected). That is mechanical, which is exactly the danger —
a real regression can hide behind adding one `join` everywhere. Mitigation: a
test helper that builds the expected destination path, so the layout is
expressed in one place and a future change does not replay 59 times.

New tests, in order of what they are worth:

| Pinned | Why |
|---|---|
| A missing source keeps its subfolder while the others prune normally | The test that matters. An unplugged drive must not delete its own backup |
| A file whose new copy fails keeps its old flat copy | A destination without room for two copies must not lose what did not fit |
| A drive root keeps its path with an empty folder, refused by that path | Otherwise the migration erases a whole-drive task's source |
| All sources missing → prune skipped entirely | The generalisation of `source_root_unreadable` |
| Two sources, two subtrees, no interleaving | The feature's central property |
| Removing a source from a task removes its subfolder on the next run | What the rejected nested-loop approach could not do |
| An empty source still materialises its folder | Otherwise it vanishes silently |
| Refusals: nested sources, duplicate folders (case-folded), source ∩ destination | The two new rules |
| `Task::sources()`: legacy folds to one, `sources` wins, repeats dropped | Modelled on the existing `destinations()` tests |
| `migrateTasks` is idempotent | The existing `destinations` test has this shape |
| A source-root `desktop.ini` becomes the subfolder's icon descriptor | The behaviour change named above |

Regression net: the current 123 Rust and 84 JS tests, once paths are updated.

## Out of scope

- No rename-based migration of existing backups.
- No per-source schedules, exclude patterns, or destinations. A task still has
  one schedule and one exclude set, and every destination receives every
  source.
- Restore is unchanged. It copies a tree and does not know about sources;
  someone restoring one source picks that subfolder as the backup path.
