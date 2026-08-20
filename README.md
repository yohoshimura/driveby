# driveby 1.6.1

A local-drive backup app with a macOS-style sidebar UI — **Tauri 2 + Rust** backend, **React 18 + Vite** frontend.

Where 1.5 was a correctness sweep, **1.6 closes the gaps 1.5 left open and makes the app shippable to other people**: restore gets the cancellation, progress and concurrency guard that backup already had; the copy loop runs files in parallel and fingerprints them as it writes, so verification no longer re-reads both sides; dates and numbers follow the app's language instead of the OS locale; the app survives its window being closed so scheduled backups actually fire; and CI builds signed bundles for Windows, macOS and Linux. Verified clean on `cargo clippy --all-targets` with **55 Rust tests and 28 frontend tests passing** as of 1.6.1.

See [1.5 and earlier](#data-loss--fixed-in-15) below for the history.

## What's new in 1.6.1

A small release with two UI decisions and the distribution follow-through 1.6 set up but never triggered.

### A quieter card during a backup

The running task card showed a detail line under the progress bar — phase, file counts, throughput, ETA. 1.6 had added it because the progress payload already carried all four and the card was throwing them away. In use it reads as noise: four figures churning at 10 Hz on a card whose bar already says how far along the run is. The line is gone and the bar stands alone.

It takes the accessibility budget with it, so the bar now carries an `aria-label` of its own — previously the phase text was what a screen reader announced. `PHASE_KEY`, the six `backup.phase.*` strings and `formatSpeed` had no other consumer and were removed rather than left behind. Nothing changed in the backend: `backup-progress` still carries `phase`, `copiedFiles`, `speedBps` and `etaSeconds`, and `ProgressContext` still stores them.

### History retained by age

`historyLimit` asked how many past runs to keep. That is not the question anyone has about a backup log — "how far back can I look" is. It becomes `historyRetention`:

| Setting | Meaning |
|---------|---------|
| `1d` / `1w` | A fixed 24 hours / 7 days back. |
| `1m` / `1y` (default `1m`) | A **calendar** month or year back, clamping the day of the month — 31 March minus a month is 28 February, not 3 March. This is the same rule the Rust scheduler applies through `checked_add_months`, so "a month" means one thing across the app. |
| `all` | No date limit. **Not** unlimited — see the cap below. |

Two decisions are worth stating outright:

- **A hard cap of 5000 entries applies under every setting, `all` included.** The count *was* what bounded `history.json`; replacing it with an age alone would let an hourly task write 8 760 rows a year into a file that is rewritten in full on every change. The cap is not exposed in Settings — it is a floor under the feature, not a knob.
- **An entry with a missing or unparseable timestamp is kept.** We do not delete what we cannot date; the cap still bounds the file.

There is no migration path, because a count and an age have none: the old `historyLimit` key is dropped from `settings.json` when it loads, and every install starts at the one-month default. The purge points are unchanged — at load, on each completed run, and immediately when the setting is shortened. No timer sweeps the list, because every completed run already triggers a purge; the only way to drift is to leave the app open for days with no backup at all.

### Distribution, actually executed

1.6 wired `release.yml` for Windows, macOS and Linux and then never fired it — no `v*` tag was ever pushed, so no artifact ever existed. 1.6.1 is the first tagged release. See [Prerequisites / Running](#prerequisites--running) for what each platform produces.

`npm run bump-version` now writes `package-lock.json` as well as `package.json` and `Cargo.toml`; that omission is why 1.6.0 shipped with a lockfile still claiming 1.5.0.

## What's new in 1.6

### Restore parity

| Gap | Fix |
|-----|-----|
| **A restore could not be stopped.** The backup pipeline threaded a `CancellationToken` through every phase; restore had none, so a restore of a large tree could only be waited out. | `RestoreState` — a single-slot registry, not the task-keyed `DashMap` backup uses, because the UI exposes one restore flow — hands out a token that `copy()` selects on with the same `biased` `tokio::select!` the backup path uses. Cancellation is byte-granular and routes through the existing cleanup, so no partial file survives. `cancel_restore` is the new command; the stop button lives in the restore overlay. |
| **Two restores could run at once.** Nothing stopped a double-click launching a second one over the same destination. | `try_begin()` refuses a concurrent restore backend-side; the frontend also holds an in-flight ref and disables every Restore button while one runs. |
| **A restore looked like nothing was happening.** Rust emitted `restore-progress` and `bridge.onRestoreProgress` existed — nothing consumed either. The window sat idle for the whole copy. | A restore overlay subscribes to the events and shows files, bytes and a progress bar. The backend emit is now throttled to 100 ms like backup's, instead of firing once per file. |

### Performance

- **Parallel copies.** `copy_phase` keeps `parallelCopies` files in flight (default 4, settable to 1/2/4/8 in Settings; **1 reproduces the old sequential behaviour exactly** — the escape hatch for spinning disks). Bounded concurrency comes from a manual window over `FuturesUnordered` rather than spawned tasks: tokio's fs calls already offload to the blocking pool, so borrowed futures get real I/O overlap without `'static` bounds, and per-file results come back as values so `PhaseStats` stays single-owner.
- **Hash during copy.** `copy_file` folds bytes into an `Xxh3` as it writes them. `verify` therefore re-reads only the *destination*, comparing against the fingerprint taken while copying — it used to read both sides, doubling the I/O of an already-slow option. Scope note: verification now covers files copied by this run; files skipped as unchanged were verified by the run that copied them.
- **Bounded history.** `history.json` had no cap and was rewritten in full on every change. `historyLimit` (default 1000, `All` still available) trims on append, at load, and immediately when lowered. *(Superseded in 1.6.1 by an age-based `historyRetention` with a hard entry cap underneath it.)*
- **No more 10 Hz whole-tree re-renders.** Live progress moved out of `AppContext` into a `ProgressContext` that only `Home`, `History` and the restore overlay consume, and the remaining context value is memoised. A running backup used to invalidate the context ten times a second and re-render everything, history table included.

**Platform-native fast copy was evaluated and rejected for 1.6.** `CopyFile2` / `clonefile` / `copy_file_range` each break hash-during-copy (forcing verification back to a source re-read), and each carries its own partial-file, cancellation and mtime semantics — three platform paths, none of them exercised off Windows until this release's CI exists. Worth revisiting in 1.7 now that it does.

### Correctness

- **Directory-level case drift is corrected.** 1.5 stopped prune deleting a file whose *name* was re-cased at source; a re-cased *folder* kept the destination's stale spelling forever, because `recase_entry`'s `with_file_name` can't reach a parent component. A `recase_dirs_phase` now runs between copy and prune, driven by the source's own directory list: it asks the filesystem for each destination directory's real spelling (`FindFirstFileW`) and renames case-only mismatches shallow-first. Windows-only, like the rest of the case machinery — with a `TODO(macOS)` where APFS will need the same.
- **Scheduler.** Hourly schedules are supported; "monthly" means a calendar month (`checked_add_months`, so Jan 31 clamps to Feb 28/29) instead of a fixed 30 days; and the retry clock persists to `scheduler.json`, so restarting the app no longer forgets that a failing task just attempted — nor re-anchors a never-run task's schedule.
- **Toasts no longer cut each other short.** The 3-second timer was never cleared, so two toasts in quick succession shared the first one's deadline.
- **An error boundary** catches a render throw instead of blanking the window.

### Locale-aware formatting

`formatTime` passed `undefined` to `toLocaleString`, which means *the OS locale* — so a French UI showed system-format dates. All formatters are now built per-language by `makeFormatters(lang)` behind a `useFormat()` hook: `Intl.DateTimeFormat` for dates and chart day labels (which were hardcoded DD/MM), `Intl.NumberFormat` for byte sizes (French gets `Ko`/`Mo`/`Go` and a decimal comma) and thousands separators. Plurals go through `Intl.PluralRules` with `.one`/`.other` keys, replacing the faked `item(s)`; the leftover untranslated literals (`Never`, `Backup`, `Task`) are keys now.

### Background operation

The scheduler runs in-process, so closing the window ended the app and with it every schedule. Both new toggles default **off** — with them off, 1.6 behaves exactly like 1.5:

- **Keep running when the window is closed** hides to a tray icon instead of quitting. The webview stays alive, so schedules, history writes and notifications keep working.
- **Start driveby at login** registers autostart and launches hidden (`--hidden`).

The tray icon itself is always created — it's the only way back to a hidden window, and creating/destroying it as a setting changes is where the platform bugs live.

### Distribution

- **CI** (`.github/workflows/ci.yml`) runs the Rust and frontend tests on Windows plus a Linux job that keeps the `#[cfg(not(windows))]` paths honest.
- **Releases** (`.github/workflows/release.yml`) build on a tag: Windows (MSI + NSIS), macOS (universal), Linux (AppImage + deb).
- **Auto-update** is wired end to end — plugin, capabilities, a Settings section with a manual check and a silent check at launch that only ever raises a toast. Signing keys are in place, so tagged releases ship signed updater artifacts; see [Update signing](#update-signing).
- **The version lives in two files** (`package.json`, `src-tauri/Cargo.toml`) instead of five: `tauri.conf.json` has no `version` key so it reads Cargo's, `main.rs` logs `CARGO_PKG_VERSION`, and the sidebar string is a `{version}` parameter fed by vite's `__APP_VERSION__`. `npm run bump-version 1.6.1` updates both manifests.

## Update signing

Updates are signed with a minisign keypair produced by `npm run tauri signer generate`. The keypair lives **outside the repository** and is never committed:

| File | Location |
| --- | --- |
| Private key | `C:\Users\Yoshimura\Keys\driveby\.driveby-updater.key` |
| Public key | `C:\Users\Yoshimura\Keys\driveby\.driveby-updater.key.pub` |

The public half is already checked in as `plugins.updater.pubkey` in `src-tauri/tauri.updater.conf.json` (key ID `0883B477825312B9`). It must stay byte-identical to the `.pub` file, or clients reject every update.

CI signs from two GitHub Actions secrets:

- `TAURI_SIGNING_PRIVATE_KEY` — the full base64 contents of `.driveby-updater.key`.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the passphrase chosen at generation time. **Required:** this key is passphrase-protected (its header declares the scrypt KDF), so signing fails without it.

The release workflow layers `tauri.updater.conf.json` in only when `TAURI_SIGNING_PRIVATE_KEY` is present, so local `npm run tauri build` keeps working untouched either way.

To rotate the keypair, regenerate it, then replace both the `.pub` contents in the updater config and the two secrets — and keep the old public key serving until every client has updated past it.

---

## 1.5 — the correctness sweep

Where 1.4 was an idiom-and-lint audit, 1.5 was a **correctness sweep**: no new feature and almost no UI surface change, but a long pass through the backend hunting behavioural bugs. Five of the fixes below destroyed user data *and reported the run as successful*.

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

## Tests (as of 1.5)

41 unit tests, up from 19 at the end of 1.4. The ones guarding a data-loss path were each confirmed to fail against the pre-fix code rather than merely passing after it:

- `prune_preserves_unreadable_subtree` — the protected file really was deleted before the fix.
- `prune_is_a_no_op_when_the_root_itself_is_unreadable` — prune emptied the destination.
- `prune_recases_instead_of_deleting_on_case_only_rename` — the destination came back empty (`left: []` against the expected `["README.md"]`).
- `prune_removes_emptied_readonly_directory`, `restore::copy_overwrites_readonly_destination` — both failed with "Accès refusé. (os error 5)".
- `prune_leaves_a_surviving_icon_folder_readonly`, `prune_never_removes_the_destination_roots_icon_descriptor` — failed on the attribute mask and on the deleted descriptor.
- `never_backed_up_task_becomes_due_one_interval_after_first_sighting`, `a_failing_task_waits_a_full_interval_before_retrying` — failed against the old `last.unwrap_or(now)` formula and against a schedule-only anchor.
- `restore_leaves_the_destination_alone_when_the_backup_is_unreadable` — drives the real `restore()` against `tauri::test::mock_app()`, with the unreadable backup file simulated by an exclusive handle (`share_mode(0)`). Reintroducing the old cleanup makes it fail on a destination file that no longer exists.

`restore` is generic over the Tauri runtime purely so that last test can pass a mock handle; production infers `R = Wry` and no call site changed. The `test` feature is a dev-dependency, so the release build is unaffected.

Two caveats worth keeping in view. `execute()` takes an `AppHandle` and is still not directly unit-testable — the phase tests are the regression net. And the scheduler evidence is unit-level only: an end-to-end daily trigger would take 24 h to observe. *(1.6 closed the first of these: `execute()` is generic over the runtime and driven end to end by `mock_app()`.)*

## What's retained from 1.4 and 1.3

The 1.4 audit (type-safe cancellation through the `CancellationToken` rather than error-string matching, the `path_contains` Windows-canonicalize fix, scheduler survival across `Mutex` poisoning, `checked_div` in the ETA) and the 1.3 EN/FR language switcher — every user-visible string flows through `useT()` and the in-tree `i18n.js`, with the language section in Settings. Also the folder-icon hash-verification phase and the external-drive app icon.

The one UI addition in 1.5: `CompletePayload` gained `unreadable`, `AppContext` stores it on the history entry, and History renders an orange line saying how many source items could not be read and that their copies were kept. New i18n key in both locales. A permanently locked folder in a source shouldn't paint every run red, so such a run still counts as a success.

## Prerequisites / Running

```bash
npm install
npm run tauri dev
npm run tauri build
```

## Open items for 1.7+

- **Per-run snapshots with restore-from-snapshot.** Deliberately deferred: backup is a live mirror with no manifest or versioning, so snapshots change the storage model (format, retention, garbage collection) and deserve their own design cycle rather than a corner of a release that was otherwise about parity and performance.
- **Platform-native fast copy**, revisited now that the CI matrix exists to exercise it — see the reasoning under [Performance](#performance).
- **Case handling on macOS.** APFS is case-insensitive by default, so `KeepSet::by_lowercase`, `fsutil::on_disk_name` and `recase_dirs_phase` should extend there; all three carry a `TODO(macOS)`.
- **A localized tray menu.** Its labels are English-only, because the menu has to be rebuilt to change them.
- **End-to-end backup tests in CI.** `execute()` is now generic over the Tauri runtime and driven by `tauri::test::mock_app()`, which is what made the parallel-copy and directory-recase regressions testable; the scheduler is still unit-level only, since observing a real daily trigger takes a day.
