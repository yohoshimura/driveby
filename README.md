# driveby 1.5

A local-drive backup app with a macOS-style sidebar UI — **Tauri 2 + Rust** backend, **React 18 + Vite** frontend.

Where 1.4 was an idiom-and-lint audit, 1.5 is a **correctness sweep**: no new feature and almost no UI surface change, but a long pass through the backend hunting behavioural bugs. Five of the fixes below destroyed user data *and reported the run as successful*. Verified clean on `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and **41/41 unit tests pass**.

> **Note on numbering:** what was previously branded `v2.x` was renumbered to `v1.x` (the actual 1.0+ shipping line, post-Tauri-rebrand). The pre-Tauri Electron snapshots that used to be `v1.x-beta` are now `v0.x-beta`. This README refers to features by the new numbers; `conversation.md` keeps the original session-time labels for historical accuracy.

> **Note on layout:** the app lives at the repository root. It used to sit under a per-release version folder (`v1.4.0/`, and before that one folder per release); those folders were flattened away in 1.5. Earlier releases (`v1.0.0`–`v1.3.0` and the `v0.x-beta` pre-Tauri snapshots) remain in git history at commit `a768e06` and can be recovered with `git checkout a768e06 -- <version-folder>`.

## Data loss — fixed in 1.5

Every one of these ended with `success: true`.

| Bug | Fix |
|-----|-----|
| **An unreadable source subtree made prune delete the destination copy.** `walk()` answered a failed `read_dir` with `skipped += 1; continue`, so the subtree vanished from `files` and therefore from prune's `keep` set. prune read that absence as "deleted at source". A transient permission error or a locked folder was enough to wipe that part of the backup. | `walk()` returns a `WalkResult` carrying an `unreadable` set — every directory whose listing failed, every entry whose type could not be read, every file that could not be stat'd. prune protects it exactly like a user exclude pattern, and skips the entry *without pushing it on the stack*, so protecting a directory protects its whole subtree. |
| **A cut-short listing of the source *root* protected nothing.** The sibling case of the above: `read_dir` failing on the root was made fatal, but the listing failing part-way through `next_entry` fell into the generic "record it and carry on" branch. It records `rel_of(root, root)`, which is `""` — and prune only ever tests non-empty entry paths, so a source drive that dropped out mid-listing left prune deleting everything it had not yet seen. | Root listing failures are fatal wherever they happen. As a second line of defence prune treats an empty path in `unreadable` as "the root is unknown" and skips entirely. |
| **Restoring a backup onto itself emptied every file in it.** With `destination == backup_path`, `dst == src` for every file, and `File::create(dst)` truncates the very file the already-open `File::open(src)` handle is about to read: 0 bytes in, 0 bytes out. Reachable from the UI in two clicks — History → Restore → pick the backup folder in the picker. | The guard the backup pipeline has had since 1.2, extracted as `fsutil::reject_overlap` and called from both. |
| **Restore deleted a destination file it never wrote to.** The loop dropped `dst` on any error from `copy`, to keep a half-written file from colliding on size during a re-run — but `copy` fails at `stat source` and `open source` *before* it reaches `File::create`. An unreadable file in the backup deleted the good file the user already had there. | Cleanup moved inside `copy`, immediately after `File::create` succeeds, which is the only point that knows the destination holds our bytes rather than the user's. |
| **A case-only rename deleted the file that had just been copied.** NTFS is case-insensitive but case-*preserving*: re-case a source file (`readme.md` → `README.md`) and `File::create` on the new spelling writes through to the existing directory entry, which keeps the old one. prune looked up the old spelling in `keep`, missed, and deleted it. Missing from the backup until the next run. | `keep` is a `KeepSet` answering `Exact` / `CaseDrift` / `Absent`, with the case-insensitive index built only on Windows. On drift the destination entry is re-spelled instead of deleted. |

## Windows attributes and folder icons

| Bug | Fix |
|-----|-----|
| **READONLY blocked two operations outright.** `apply_attrs` mirrors READONLY onto the destination, so the tree accumulates read-only files and directories. Measured against std on NTFS: `remove_file` succeeds, but `File::create` and `remove_dir` are both `PermissionDenied`. Emptied custom-icon folders could therefore never be pruned, and one read-only file already in the chosen destination aborted an entire restore. | `fsutil::clear_readonly` before both, and before the copy pipeline's `remove_file`/`File::create` too — unreachable on NTFS, but the std fast path needs `FileDispositionInfoEx`, which exFAT does not support, and exFAT is the common format for the external drives this app exists to write to. |
| **prune stripped `+R` from folders it wasn't removing.** The bit was cleared for every directory walked, and only `mirror_dir_attrs_phase` — two phases later — puts it back. `verify_icons_phase` sits in between and returns early on cancellation, so pressing Stop left every custom-icon folder in the destination rendering with the default icon. | prune tries the removal first and only reads attributes, clears the bit and retries on failure — restoring what it read if the directory turns out not to be empty. |
| **The destination root's icon depended on the source having one.** prune's contract says a root-level `desktop.ini` is never removed from the destination, but the guard read from the `excluded` set, which the *source* walk fills in. Back up a source whose root has no `desktop.ini` and the backup drive lost its custom icon on the first run. | prune applies `is_root_icon_marker` to the destination entry directly. Nested descriptors keep tracking the source like any other file. |

## Scheduler

| Bug | Fix |
|-----|-----|
| **A task that had never run never fired.** `seen` was a `HashSet<String>`: it remembered *that* a task had been observed, not *when*. The due check fell back to `last.unwrap_or(now)`, so the reference point was recomputed as `now` every tick, and `now - now` is never `>= interval`. A scheduled task had to be backed up by hand once before the scheduler would ever touch it. | `seen` holds the moment this process first observed each task, and the decision moved into a pure `is_due`. The tick that first observes a never-run task records the timestamp and returns without firing, so launching the app still doesn't double as a backup trigger. Entries for deleted tasks are dropped each tick. |
| **A persistently failing task retried every 60 seconds forever.** `lastBackup` is only written on success, so that partial failures don't reset the schedule clock. Once a task crossed its interval and started failing, no anchor advanced and it came due on every tick — each attempt adding a history row and a notification. | The per-process map carries a `TaskClock` (first sighting + last attempt) and attempts are spaced by the same interval as successful runs. The catch-up case is unaffected: a task last backed up days ago has no recorded attempt and still fires as soon as the scheduler notices it. |

## Restore brought up to parity

Restore was a second-class citizen of the same codebase. It never called `long_path()`, so a tree the backup pipeline handled happily could not be restored once any path crossed the 260-character limit — now applied to `metadata`, `read_dir`, `File::open`, `File::create`, `create_dir_all` and `set_file_mtime`. It never mirrored file attributes either, so a restored tree came back with default icons; it now calls `fsutil::apply_attrs` the way the copy path does.

## Performance

`glob::matches()` called `Regex::new` for every pattern on every path, from both the source walk (once per directory entry) and the prune pass (once per destination entry) — on the order of a million compilations per run for five patterns over a 100k-file tree, all producing the same handful of automata. A `PatternSet` is now compiled once in `execute()` and borrowed by both walkers. Measured on 50 000 paths × 5 patterns, release build, old implementation against new in the same process with an assertion that both classify every path identically: **29.25 s → 9.48 ms**.

Matching semantics are unchanged and deliberately so: each rule is still tried against the full relative path *and* the basename, the last matching rule still wins so `!keep.tmp` re-includes, and a pattern that fails to compile is still dropped rather than failing the run.

## Structure

`execute()` was ~390 lines running preflight, walk, copy loop, prune, icon hash verification, folder attribute mirroring, progress emission and optional verification, with a dozen `let mut` counters live across the whole span. It is now split into phases against a shared `RunCtx` and one `PhaseStats` accumulator:

| Phase | Lines |
|-------|-------|
| `preflight` | 24 |
| `copy_phase` | 88 |
| `prune_phase` | 31 |
| `verify_icons_phase` | 61 |
| `mirror_dir_attrs_phase` | 36 |
| `verify_phase` | 8 |
| `execute` (orchestration + payload) | 77 |

`RunCtx` owns both emit paths — `emit_phase()` for one-shot boundary events, `maybe_emit()` for the throttled copy stream — which retired the free-standing `maybe_emit` and its `#[allow(clippy::too_many_arguments)]`. Behaviour is unchanged by construction: same phase order, same cancellation points, `started` still taken after the walk so `duration_ms` still excludes walk time.

`src-tauri/src/fsutil.rs` was added to hold the cross-platform helpers both pipelines need (`long_path`, `read_attrs`/`apply_attrs`, `path_contains`, `reject_overlap`, `clear_readonly`) plus a shared `rel_of()` replacing four copies of the strip-prefix-and-normalize pair.

## Tests

41 unit tests, up from 19 at the end of 1.4. The ones guarding a data-loss path were each confirmed to fail against the pre-fix code rather than merely passing after it:

- `prune_preserves_unreadable_subtree` — the protected file really was deleted before the fix.
- `prune_is_a_no_op_when_the_root_itself_is_unreadable` — prune emptied the destination.
- `prune_recases_instead_of_deleting_on_case_only_rename` — the destination came back empty (`left: []` against the expected `["README.md"]`).
- `prune_removes_emptied_readonly_directory`, `restore::copy_overwrites_readonly_destination` — both failed with "Accès refusé. (os error 5)".
- `prune_leaves_a_surviving_icon_folder_readonly`, `prune_never_removes_the_destination_roots_icon_descriptor` — failed on the attribute mask and on the deleted descriptor.
- `never_backed_up_task_becomes_due_one_interval_after_first_sighting`, `a_failing_task_waits_a_full_interval_before_retrying` — failed against the old `last.unwrap_or(now)` formula and against a schedule-only anchor.
- `restore_leaves_the_destination_alone_when_the_backup_is_unreadable` — drives the real `restore()` against `tauri::test::mock_app()`, with the unreadable backup file simulated by an exclusive handle (`share_mode(0)`). Reintroducing the old cleanup makes it fail on a destination file that no longer exists.

`restore` is generic over the Tauri runtime purely so that last test can pass a mock handle; production infers `R = Wry` and no call site changed. The `test` feature is a dev-dependency, so the release build is unaffected.

Two caveats worth keeping in view. `execute()` takes an `AppHandle` and is still not directly unit-testable — the phase tests are the regression net. And the scheduler evidence is unit-level only: an end-to-end daily trigger would take 24 h to observe.

## What's retained from 1.4 and 1.3

The 1.4 audit (type-safe cancellation through the `CancellationToken` rather than error-string matching, the `path_contains` Windows-canonicalize fix, scheduler survival across `Mutex` poisoning, `checked_div` in the ETA) and the 1.3 EN/FR language switcher — every user-visible string flows through `useT()` and the in-tree `i18n.js`, with the language section in Settings. Also the folder-icon hash-verification phase and the external-drive app icon.

The one UI addition in 1.5: `CompletePayload` gained `unreadable`, `AppContext` stores it on the history entry, and History renders an orange line saying how many source items could not be read and that their copies were kept. New i18n key in both locales. A permanently locked folder in a source shouldn't paint every run red, so such a run still counts as a success.

## Prerequisites / Running

```bash
npm install
npm run tauri dev
npm run tauri build
```

## Open items for 1.6+

- **Restore has no cancellation.** The backup pipeline threads a `CancellationToken` through every phase; restore has none, so a restore of a large tree can only be waited out. Needs a `RestoreState`, a `cancel_restore` command and a stop button — deliberately left out of the 1.5 sweep rather than half-done.
- **Directory-level case drift is not corrected.** 1.5 stops prune from deleting a file whose *name* the user re-cased at source, and re-spells it; a re-cased *folder* keeps the destination's old spelling. No data is lost either way.
- Locale-aware date/number formatting via `Intl.DateTimeFormat` / `Intl.NumberFormat` driven by the active language.
- Parallel copies, platform-native fast copy (`clonefile` / `copy_file_range` / `CopyFile2`), streaming hash-during-copy, per-run snapshots with restore-from-snapshot, history virtualization.
