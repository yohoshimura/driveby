# BackupDrive — Conversation Log

A compressed record of the decisions and changes made across this project's evolution from 0.0.1 (pre-rename 1.0.1) up through the current 1.4 release. Full raw transcript lives in the local Claude Code project log.

## Timeline

### 0.0.1 – 0.4 (existing — pre-rename "1.0.1 – 1.4")
Sequence of standalone Electron + React builds, each iterating on UX.

### 0.5 — Electron, corrections pass
- Deep audit of 1.4 → `claude_report.md` at repo root (23+ items: correctness, security, arch, UX).
- Rewrote all of 1.5 addressing every audit item.
- Reworked UI with macOS (Apple HIG) style inspired by [macos_ui](https://github.com/macosui/macos_ui): SF Pro fonts, 8 system accent colors, 0.5px hairlines, green toggles, light/dark/system mode.
- Removed unused `@fontsource` packages after switching to native Apple fonts.

### 0.6 — Tauri + Rust rewrite
Full port: React UI kept, Electron `main.js` replaced by Rust backend. ~10× smaller installer, lower memory, safer fs ops.

**Rust backend (`v0.6.0-beta/src-tauri/`):**
- `main.rs` — Tauri 2 Builder, 8 IPC commands.
- `backup.rs` — tokio-based copy engine, `CancellationToken` cancellation via `tokio::select!`, per-file throttled progress emit, safe timestamped folders, regex-guarded cleanup.
- `glob.rs` — `*`, `**`, `?`, `!neg` with comma/newline separators + tests.
- `persist.rs` — atomic JSON read/write (tmp + rename).
- `DashMap` for concurrent active-backup tracking, `uuid v4` IDs, `serde` camelCase payloads.

**React frontend (`v0.6.0-beta/src/`):**
- New sidebar layout (search + icon nav groups + brand footer), inspired by macos_ui screenshot.
- `App.jsx`, `Sidebar.jsx`, `Toolbar.jsx` (sidebar toggle + title).
- Ported 1.5 components to `.jsx`: Home, TaskCard, NewTaskForm, History, Settings, ConfirmDialog, Toast, common/*.
- `lib/tauri.js` adapter — replaces `window.electron` with `invoke` / `listen` / plugin-dialog / plugin-opener / plugin-notification.
- `context/AppContext.jsx` adapted for async `listen()` (Promise<UnlistenFn>).
- `EmptyState` default text is just **"Empty"** per user request.
- Scheduler still in React layer (daily/weekly/monthly triggers).

### Post-0.6 tweaks

**Build fixes**
- Removed erroneous `[lib]` stanza from `Cargo.toml` (no `lib.rs`).
- Generated placeholder 1024×1024 icon via PowerShell, then ran `npx @tauri-apps/cli icon` to produce full icon set.

**Scroll fix**
- Classic flexbox overflow bug: `.content` (flex child with `overflow-y: auto`) wouldn't scroll. Added `min-height: 0` to `.main`, `.content`, `.sidebar__nav`. Wrapped content in `.content__inner` for max-width column while keeping full-width scrollbar.

**Stat visualizations**
- Replaced 3 stat bubbles on Home with SVG charts:
  1. **PieChart** — donut of success/error/cancelled bytes; center shows formatted total, shows `0` (not `0 B`) when empty, no "Saved" inner label.
  2. **ProcessCycle** — accent-colored ring with arrow + one dot per task, count in center.
  3. **Speedometer** — half-circle gauge. Needle fixed at 90° (center). Left half translucent red (intensity reflects failures), right half translucent green (intensity reflects successes). No inner text.

**Translucency + accent tint**
- Surface opacities bumped ~0.7 → ~0.92.
- All translucent surfaces tinted with active accent via `color-mix(in srgb, var(--accent) N%, …)` — tint updates live with accent change.

**Accents + naming**
- Added 4 accents: **Indigo, Mint, Teal, Brown** (total 12: Blue, Indigo, Purple, Pink, Red, Orange, Yellow, Green, Mint, Teal, Brown, Graphite).
- Renamed **Preferences → Settings** in sidebar nav and toolbar title.

**Repo hygiene**
- Added repo-root `.gitignore` covering `node_modules/`, `**/target/`, `dist/`, `src-tauri/gen/`, env files, OS/editor junk.

### 0.7 — reliability + incremental + restore
Built on the 0.7 audit — implemented the "big wins" block.

**Backend**
- `backup.rs`: manifest.json per dated folder; hardlink-based incremental (size+mtime match against the previous successful backup); xxHash3 verify pass; retry with exponential backoff (3×); continue-on-error (collect failures into `errors.log` and fail only if policy requires); mtime preservation via `filetime`; adaptive copy buffer (256 KB < 4 MiB < 1 MiB); cleanup parses the ISO timestamp from folder names instead of using folder mtime.
- `restore.rs`: `read_manifest` + `restore(backup_path, destination)` with `restore-progress` events.
- `scheduler.rs`: background tokio task polling tasks.json every 60 s; fires due daily/weekly/monthly tasks even when the window is closed; writes back `lastBackup` atomically.
- `main.rs`: new plugins — `tauri-plugin-single-instance`, `tauri-plugin-window-state`, `tauri-plugin-updater` (config stub); `tracing` + `tracing-appender` rotating daily log files in `app_log_dir`; new commands `read_manifest`, `restore_backup`, `reveal_logs_folder`.

**Frontend**
- Dropped `useScheduler` (now in Rust).
- `AppContext`: listener subscribes once; notification logic pulled from a `settingsRef`; new `restoreBackup` helper prompts for destination and confirms.
- `History`: **Restore** button (for successful entries) alongside Reveal/Delete.
- `Settings`: new **Backup Options** group (Incremental / Verify / Continue on error / Preserve mtime) and a **Diagnostics** group with "Open Logs…".
- `App.jsx`: persists `sidebarOpen` and `lastView` to settings; restores on launch.
- New settings: `incremental`, `verify`, `continueOnError`, `preserveMtime`, `sidebarOpen`, `lastView`.

### 0.8 — correctness pass (folded into 1.0)
Built on the 0.7 audit — shipped the **Critical** block only.

- **`lastBackup` race fixed.** Ownership moved into Rust (`backup::update_last_backup`). Rust writes `tasks.json` atomically + emits `task-updated`. Scheduler no longer rewrites the file. UI drops its auto-save effect and persists `tasks.json` only on user actions (add/edit/delete). UI listens for `task-updated` and reconciles without re-saving.
- **Incremental base validation.** Previous-backup selection now requires `manifest.json` to parse and `failed_files == 0`. Partial/corrupt folders are skipped.
- **Cross-volume hardlink guard.** `volume_id()` helper (Unix `MetadataExt::dev`, Windows `MetadataExt::volume_serial_number`). Mismatch short-circuits the hardlink path — no wasted per-file attempts.
- **Windows long paths.** `long_path()` helper prefixes `\\?\` (or `\\?\UNC\`) for absolute paths; used at every fs call site.
- **Durability.** `writer.sync_all()` after flush on every copy; manifest `sync_all`; Unix directory fsync at end of run.
- **Scheduler simplified.** Removed the post-completion `tasks.json` rewrite block — `run_backup` handles it.

**New event:** `task-updated` (Task JSON). **New bridge method:** `onTaskUpdated(cb)`.

### 1.0 — sync rewrite, rebrand to driveby, Statistics view, polish

This release pivoted the backup model and rebranded the app. Active folder is now `v1.0.0/`; all earlier folders were renamed `v0.0.1-beta` … `v0.7.0-beta` (plus the former `v1.8.0` (now folded into `v1.0.0`) which was promoted into this release). The repo folder itself was renamed `backup-drive` → `driveby` (then reverted at end of session — see Session-end note).

**Rebrand**
- App name **BackupDrive → driveby** everywhere user-facing: `package.json` (`"name": "driveby"`, `"version": "1.0.0"`), `Cargo.toml` (package + `[[bin]]` name `driveby`, version 1.0.0), `tauri.conf.json` (productName, identifier `com.driveby.app`, window title, longDescription), `index.html` `<title>`, `styles.css` header, `Sidebar.jsx` brand block ("driveby — Version 1.0"), `NewTaskForm.jsx` schedule hint, `AppContext.jsx` notification title, `capabilities/default.json`, log filename `driveby.log`, startup `info!("driveby 1.0 starting")`, root README + v1.0.0/README.

**Backup engine — major behavioral pivot (`backup.rs`)**
- **Sync into destination directly** — no `<name>_<timestamp>` wrapper folder, no `manifest.json` written. Files matching destination by size + mtime are skipped (`unchanged_files` counter).
- **Mirror-delete pass** — new `prune_destination(target, source_paths_set, token, &mut deleted)` walks dest, removes orphan files, then bottom-up removes now-empty directories. Emits a `phase: "pruning"` progress event. `cleaned` field of `CompletePayload` reports the count.
- **Windows custom folder icons preserved.** New Windows-only helpers `read_attrs(p)` / `apply_attrs(p, attrs)` using `windows-sys` (added as Cargo dep, target-cfg gated). Mask = `READONLY | HIDDEN | SYSTEM` only — never propagates ARCHIVE/REPARSE_POINT. `walk()` now returns `(files, dirs, total, skipped)` so directory paths can be rewalked at end. After every file copy → apply_attrs to dest. After prune → loop over `dirs` and apply_attrs to corresponding destination dirs. Without this, `desktop.ini` lost Hidden+System and parent folders lost the System bit, so Explorer ignored the icon descriptor.
- **Per-file progress no longer overshoots on retries.** `copy_with_retries` callback signature changed from delta-bytes to *cumulative-bytes-this-file*; on retry it calls `on_progress(0)` to reset. Caller maintains `base_bytes` snapshot per file. Without this, retried chunks accumulated and progress hit 100% while loop was still running ("stuck at 100%" symptom).
- **Forced final 100% emit** before completion (throttled `maybe_emit` could swallow the last update).
- **`lastBackup` always recorded on non-cancelled completion** (was `if payload.success` only). Timeout raised 5 s → 15 s. Long runs that hit a recoverable error still record their timestamp.
- **Tracing** `info!("emitting backup-complete")` for diagnosis.
- **Removed dead code** from the v1.7→v1.8 rewrite: `find_previous_backup`, `cleanup_old_backups`, `sanitize_name`, `sync_dir`, `VolumeId`, `volume_id`, the entire hardlink path, manifest writing, `errors.log` write, and unused imports (`DateTime`, `Regex`, `HashMap`, `debug`). Kept `MANIFEST_NAME`, `ERRORS_LOG`, `Manifest` for `restore.rs::read_manifest` backward-compat with v1.7 dated folders.
- `CompletePayload` field `hardlinked → unchanged`. AppContext history entry updated.
- Settings: removed `incremental` toggle from UI and "Maintenance" group (autoCleanupDays). The `incremental` field stays on the Rust struct for forward-compat with old `settings.json`. Accent color picker removed; data-accent locked to `'blue'` in AppContext (so previously-saved accent values are ignored).

**Statistics view (new top-level tab)**
- New `Statistics.jsx` route between History and Settings. Sidebar item with `BarChart3` icon. App.jsx routes + Ctrl+3 shortcut (Settings moved to Ctrl+4). TITLES updated.
- Three new chart components in `src/components/charts/`:
  - **`CandleChart.jsx` ("Backed Up")** — went through several iterations on user request: candle → bar → line → scatter → histogram → **stacked cumulative area chart** (final). x = run date `dd/mm`, y = cumulative bytes (auto-formatted via `formatBytes`), three stacked bands (cancelled/error/success bottom-up), accent line traces grand total, dot per run, legend below, horizontal scroll, auto-scrolls to latest. `chartHeight = 340`.
  - **`TaskList.jsx` ("Tasks")** — vertically scrollable list (max-height 240px) with name, source→destination, last-run time, schedule.
  - **`GroupedBarChart.jsx` ("Successful Runs")** — horizontally scrollable grouped vertical bars per task, success (accent) next to error (red). `BAR_W=28`, `GROUP_GAP=80`, `MIN_WIDTH=720` (expand horizontally per user). Y-axis ticks de-duplicated via `Set` so `maxCount=1` shows `0,1` not `0,1,1,1,1`. Gridlines positioned by tick value. Task names full (no `…` truncation), 11px.
- Old PieChart / ProcessCycle / Speedometer files left in `charts/` but no longer imported (tree-shaken). Stats section removed from `Home.jsx`.
- Statistics CSS in `styles.css`: `.stat-block`, `.candles*`, `.task-list-stat*`, `.grouped-bars*`, `.chart-empty`, `.legend-dot`, `.candles__legend`.

**Settings UI redesign**
- All `.setting-row__hint` description divs removed; replaced by an `i`-bubble tooltip on the left of each control.
- New `common/InfoTip.jsx` component with `placement` prop (`'left'` default, `'right'` for stacked rows). Renders a 16px circle with italic `i` glyph; `data-tip` attribute drives a CSS pseudo-element tooltip.
- Tooltip dialog: translucid frosted panel — `color-mix(bg-primary 78%, accent 12%)`, accent-tinted hairline border, `backdrop-filter: blur(20px)`, `label-primary` text. (Went through several iterations chasing "translucid" vs "blue enough" — landed back on the accent-mixed bg-primary version.)
- Info bubbles excluded from Appearance and Diagnostics rows on user request.
- `.setting-row__control { min-width: 72px; justify-content: flex-end; }` so all toggle+info pairs occupy the same right-aligned column block (16+10+36+padding = 72 px), giving symmetric alignment across General and Backup Options groups.
- Diagnostics button label "Open Logs…" → "Open".

**Other UI tweaks**
- TaskCard: removed `formatSpeed`/`formatDuration` imports + speed/ETA/dot-separators from the right-side stat line. Only `progress%` remains during a run. Primary action label flipped "Back Up" → "Start" → "Back up".
- Toast: `bg-window-solid` mixed with `transparent` (55%) + `backdrop-filter: blur(20px)` + softened hairline ring → translucent pill.
- `Sidebar.jsx`: NAV section name "App" → "Application".
- `Home.jsx`: New-task button label "+ New Task" → "New task".
- Autofill disabled on every text-entry surface — `autoComplete="off"` (+ `autoCorrect`/`autoCapitalize`/`spellCheck={false}` and unique `name`s on writable fields) on inputs in NewTaskForm (Name, Source, Destination), Sidebar search, History search, Settings (Default destination + Exclude patterns textarea).

**Versioning + cleanup**
- All older folders renamed with `-beta` suffix via `git mv` (or `mv` fallback): `v1.0.1` … `v1.7.0` → `v0.0.1-beta` … `v0.7.0-beta`. `v1.8.0` → `v1.0.0`.
- Repeated cleanup of build artifacts (`node_modules`, `src-tauri/target`, `src-tauri/gen`).
- Removed dead-code warning: `fn incremental` accessor deleted from `impl Settings`.

**Session-end housekeeping**
- The repo folder was renamed `backup-drive → driveby` externally; we then attempted to undo it back to `backup-drive`. Inside the running session the rename failed (Windows refused — `driveby` is the cwd of this Claude Code process). An empty `backup-drive` stub was cleaned up. The user must finish the rename from outside the session: `Rename-Item driveby backup-drive` in PowerShell, plus rename the Claude project state dir `~/.claude/projects/C--Users-Yoshimura-Documents-Github-driveby` → `…-backup-drive` so the JSONL transcript `e048a705-0e65-40a7-8cc1-450250038fc5.jsonl` keeps pairing.

### 1.1 — motion + Modify Task

`v1.1.0/`. Built on top of 2.0's sync engine; surface-level work, no backend behavioural change.

- **Modify action.** `TaskCard` got a `Modify` button that disables while a backup is running and reuses `NewTaskForm` in edit mode. `Home.jsx` introduced an `editingId` state and an `useExitTransition`-backed mount/unmount so the form animates in/out cleanly. New `editTask(id, patch)` in `AppContext` writes through `bridge.saveTasks`; `lastEditingRef` keeps the form populated while it's animating away. Toast `"Task updated"`.
- **Motion system.** `cubic-bezier(0.32, 0.72, 0, 1)` easing throughout. Route cross-fade in `App.jsx` (`<div className="view-route" key={view}>`); staggered task list mount via `--stagger` CSS var; sidebar item pop + icon scale; animated progress bar with shimmer; button press-scale + hover lift; tooltip fade/scale; Statistics chart mount-in (area path fade, bars grow upward, list rows stagger). Durations 100–340 ms.
- **`prefers-reduced-motion`.** Single media-query block in `styles.css` collapses every animation/transition to ~0 ms.
- **Sidebar/version label.** "driveby — Version 1.1".

### 1.2 — correctness pass + folder-icon round-trip + new app icon

Triggered by an audit request: "analyse v1.1.0 to find some flaws". The audit surfaced 17+ issues across the Rust backup engine, the React UI, and dead code from the 1.0-line hardlink-incremental flow. v1.2.0 addresses every actionable item, plus a new app icon and a folder-icon-mirroring fix that came in as separate follow-ups.

**Concurrency / data-loss fixes (Rust):**
1. **Atomic concurrent-run guard.** `BackupState::register` (which blindly inserted into a `DashMap`) replaced by `try_register`, which uses `dashmap::Entry::Vacant` and returns `None` if a token already exists for that task. `run_backup` returns "A backup is already running for this task" instead of letting two `execute()` futures stomp the same destination.
2. **Exclude vs prune.** `walk()` now also returns `excluded: HashSet<String>` (relative paths matched by user patterns or by the new root-icon-marker rule). `prune_destination` accepts both that set *and* the patterns themselves and skips matching destination entries entirely. Adding `node_modules` to excludes no longer wipes a pre-existing `node_modules` from the destination.
3. **Source/destination overlap rejection.** New `path_contains(parent, child)` helper (case-insensitive on Windows, canonicalisation pass) called twice in `execute()` to reject self-syncs and any nested overlap before any I/O.
4. **2-second mtime tolerance.** `same_mtime` was strict whole-seconds equality, which meant exFAT/FAT destinations (which round to even seconds) re-copied every file every run. Now `(a.as_secs() - b.as_secs()).abs() <= 2`. Test added.
5. **Restore durability.** `restore::copy()` calls `sync_all`, preserves mtime via `filetime::set_file_mtime`, surfaces I/O errors from `walk()` (the v1.1.0 `while let Ok(Some(entry))` pattern silently truncated on failure), and unlinks half-written files on failure so a re-run isn't tricked by size-collision.
6. **`tasks.json` write-race.** New `persist::with_tasks_lock` (`tokio::sync::Mutex`) wraps every read-modify-write across both `save_tasks` (JS-driven) and `update_last_backup` (Rust-driven). `lastBackup` is now also only persisted on `payload.success`, not on any non-cancelled completion — partial-failure runs no longer reset the schedule clock.
7. **`continueOnError` honoured for `create_dir_all`.** Failure on a parent dir bumps `failed_files` and continues instead of `?`-propagating.
8. **Final-failure cleanup in `copy_with_retries`.** `fs::remove_file(long_path(dest))` runs before returning the final `Err` so a partial doesn't survive into the next sync.
9. **No-thundering-herd scheduler.** `scheduler::spawn` keeps a `Mutex<HashSet<String>>` of task IDs observed in this process. First observation of a `last_backup == None` task starts the clock at "now" instead of the Unix epoch, so a fresh install doesn't fire every daily task 10 s after launch.
10. **Destination root keeps its icon.** New `is_root_icon_marker(rel_str)` matches a top-level `desktop.ini` (case-insensitive, no `/` in the relative path); `walk()` adds it to `excluded` instead of the copy list, so it's neither propagated to the destination root nor pruned from there.
11. **Per-subfolder icons round-trip.** Three compounding bugs in the post-copy directory-attribute mirror loop: `SetFileAttributesW`'s BOOL return was ignored (now logged with `GetLastError`); empty source subfolders had no destination counterpart so `apply_attrs` silently no-op'd (now `fs::create_dir_all(long_path(&dest_dir))` runs first); iteration order was walk order so a parent's `+R` could block a child's mutation (now sorted deepest-first by `rel.len()`).

**JS / UI fixes:**
12. **Single-slice pies render as `<circle>`.** `arcPath(start=0, end=2π)` is degenerate (start point equals end point); `CandleChart.jsx` now branches on `day.slices.length === 1` and emits a full circle.
13. **Stable keybindings in `App.jsx`.** Wrapped the bindings array in `useMemo([], [])` so the `useKeyboard` effect doesn't re-attach the keydown listener every render.
14. **`NewTaskForm.submit` simplified.** The `if (ok !== false && !ok?.then) return; return;` on the edit branch was dead — `onSave` always returns synchronously. Now just `onSave(task); return;`.
15. **No dangling confirm promises.** `confirm()` previously dropped the prior `resolve` on the floor when called twice; it now calls `prev?.resolve(false)` inside the state updater before replacing it. `handleConfirm` does the same when settling.
16. **Accent picker removed entirely.** The CSS palette stays (12 swatches), but `Settings.jsx` only exposes Theme. `data-accent` is hardcoded to `DEFAULT_ACCENT` (`'blue'`) in `AppContext`. The original audit added a 12-swatch picker; the user then asked to remove it and lock to blue, so the picker UI was deleted but the underlying values kept for forward-compat.
17. **Dead-code prune.** Removed `Manifest`, `read_manifest` (Tauri command + `bridge.readManifest` wrapper), `incremental_from`, `MANIFEST_NAME`, `ERRORS_LOG`, `Settings._rest: serde_json::Value` (the `#[serde(flatten)]` field was unused and `Default::default()` on it produced an invalid map), `auto_cleanup_days`, `incremental` setting fields and their JS mirrors. README's "hardlink incremental / manifest.json safety" claims also gone.

**App icon refresh.** User supplied a flat orange external-drive-with-down-arrow JPG (`backup-drive-icon-vector_872227-104-1291922728.jpg`). PowerShell `System.Drawing` script in-session re-rendered it onto a 1024×1024 white-background PNG; `npx tauri icon ./src-tauri/icon-source.png` regenerated every variant (root `.png`/`.ico`/`.icns`, all `Square*` for Windows store, full iOS `AppIcon-*` set, all Android `ic_launcher*`, master `source-1024.png`). `tauri.conf.json`'s icon list was already pointing at these filenames, so no config change. Working `icon-source.png` deleted afterwards. Subsequent cleanup pass deleted `node_modules/` (114 MB), `src-tauri/target/` (3.1 GB), `src-tauri/gen/` (373 KB) — all .gitignored.

**Bug surfaced during dev:** initial `cp` of v1.1.0 → v1.2.0 dropped `useKeyboard.js`. Vite reported `Failed to resolve import "./hooks/useKeyboard"`. Restored the file from v1.1.0; no other follow-ups.

### 1.3 — EN/FR language switcher

`v1.3.0/`. Pure-JS i18n; no new dependencies.

- **`src/lib/i18n.js`** — flat key namespace, two locales (`en`, `fr`). `translate(lang, key, params)` does the `{name}` replace. `SUPPORTED_LANGUAGES`, `LANGUAGE_LABELS`, `DEFAULT_LANGUAGE` exported. Fallback chain: requested locale → `en` → key itself. No build-time codegen.
- **`src/hooks/useT.js`** — `useT()` returns a `useCallback`'d `t` bound to the current `settings.language`. Components do `const t = useT(); t('view.tasks')`. Inside `AppContext` itself the provider can't use the hook, so a local `tr()` helper reads from a `settingsRef` and calls `translate()` directly — that way async event listeners (the `backup-complete` toast in particular) always pick up the active language without re-binding.
- **Settings — Language section.** New segmented picker between Appearance and Diagnostics, two buttons (English / Français). Writes `language` via `updateSetting`, which round-trips through `bridge.saveSettings` and re-renders every consumer of `useT`.
- **Persistence.** `language: "en"` added to `default_settings()` in `src-tauri/src/main.rs` and `DEFAULT_SETTINGS` in `AppContext.jsx`. Validated against `SUPPORTED_LANGUAGES` at the read site so an unknown stored value falls back to `en` instead of crashing.
- **Translation coverage.** Every user-visible string in `App`, `Sidebar` (sections, items, search placeholder + aria, brand version, region aria), `Toolbar`, `Home`, `TaskCard` (last-run line interpolated, all four buttons + their aria-labels, schedule labels), `NewTaskForm` (every label/placeholder/option/error/dialog title), `Settings` (every section header, every label, every InfoTip, theme options, log button + toast), `History` (header, search, filter labels + `<select>` options, all column headers, status badges, all three row actions, empty state), `Statistics` (block headers, both chart `aria-label`s, the `<title>` tooltips inside `GroupedBarChart`, both empty states, legend), `ConfirmDialog` (Cancel + OK fallback). Brand "driveby" intentionally untranslated.
- **Behavioural shape.** No re-mount when language changes — the picker just bumps `settings.language`, every `t()` consumer reads it, React re-renders. Sidebar item search filtering already operates on the localised label list because the `useMemo` depends on `t`.
- **Sidebar version label.** Bumped to "Version 1.3".

### 1.3 patch — folder-icon hash verification

User reported that after 1.2's per-subfolder icon fix, destinations still occasionally rendered with default icons. The 2.2 fix was correct as far as it went (deepest-first attr application, dest-dir create-before-apply, `SetFileAttributesW` failure logging) but couldn't guarantee that the `desktop.ini` *bytes* on the destination matched the source. Added three layers in `v1.3.0/src-tauri/src/backup.rs`:

1. **Folder-icon descriptors are never skipped by the size+mtime fast path.** New `is_icon_descriptor(rel_str)` helper — case-insensitive `desktop.ini` basename match at any depth. The skip clause in `execute()`'s main copy loop guards on `if !is_icon_descriptor(&file.rel) { ... }` so every `desktop.ini` is re-copied on every run, even when size+mtime would normally short-circuit. Cost is negligible — these files are tiny (typically <1 KB).

2. **Post-copy `verifying-icons` phase.** After the main file-copy loop and the prune pass, but *before* the parent-folder attribute loop, every `desktop.ini` under the source is hashed via `hash_file()` (xxh3) and compared to its destination twin. Mismatches are logged at `WARN` level and force-re-copied through `copy_with_retries`. Outcome is summarised in tracing as `"re-synced N folder-icon descriptor(s) after hash mismatch"` or `"verified N folder-icon descriptor(s) — all match source"`. New progress emit with `phase: "verifying-icons"` so the UI could show this if it wanted.

3. **Read-after-apply attribute check in the dir-attrs loop.** After `apply_attrs(&dest_dir, src_attrs)` runs, `read_attrs(&dest_dir)` is called and the `Readonly|Hidden|System` mask is compared back. Mismatches emit a per-folder `WARN` with `want=` and `got=` hex masks plus the relative path, and an aggregate `attr_drift` counter logs a single summary warning at the end. This is what surfaces filesystem-level limitations like exFAT not supporting per-folder `+R` — previously these would silently produce default icons.

The hash-verify and attr-verify steps both run before the final 100 % progress emit, so a successful completion provably means every folder-icon descriptor and every parent-folder attribute matches the source. The root-level `desktop.ini` exclusion from 2.2 still applies — a top-level `desktop.ini` is added to `excluded` by `walk()` and therefore never enters the main copy loop or the verification list.

Test added: `icon_descriptor_matches_desktop_ini_at_any_depth` covers root-level, nested, case variants, and lookalike negatives (`desktop.ini.bak`, `not-desktop.ini`, `desktop_ini`).

### 2026-04-28 — version-folder renumber

Folder layout reshuffled to match the conceptual phasing of the project: the `v1.x-beta` Electron snapshots are now `v0.x-beta` (pre-1.0 betas), and the `v2.x` Tauri-rebrand series is now `v1.x` (the actual 1.0+ shipping line).

| Before        | After          |
|---------------|----------------|
| `v1.0.1-beta` | `v0.0.1-beta`  |
| `v1.1.0-beta` | `v0.1.0-beta`  |
| `v1.2.0-beta` | `v0.2.0-beta`  |
| `v1.3.0-beta` | `v0.3.0-beta`  |
| `v1.4.0-beta` | `v0.4.0-beta`  |
| `v1.5.0-beta` | `v0.5.0-beta`  |
| `v1.6.0-beta` | `v0.6.0-beta`  |
| `v1.7.0-beta` | `v0.7.0-beta`  |
| `v2.0.0`      | `v1.0.0`       |
| `v2.1.0`      | `v1.1.0`       |
| `v2.2.0`      | `v1.2.0`       |
| `v2.3.0`      | `v1.3.0`       |

Done via `git mv`. The last move (`v2.3.0` → `v1.3.0`) was blocked by a running Vite dev server on port 1420 holding `node_modules`/watching `src`; the user finished it from outside the dev session after stopping the server. Internal version strings inside each folder (`package.json`, `Cargo.toml`, `tauri.conf.json`, README, sidebar label, `info!()` startup line) were then *also* re-tagged in a follow-up pass so the codebase matches the directory layout: pre-Tauri Electron snapshots now report `0.x-beta`, the Tauri-rebrand series now reports `1.x`. Earlier sections of this log still refer to the *original* folder/version names ("driveby 2.0 starting", `v2.1.0`, etc.) because that's how things were named at session time; the renumber row in this section is the canonical mapping.

### 1.4 — Rust audit pass

Triggered by an explicit checklist review request covering safety, idiom, ownership, errors, performance, modules, tests, logging, deps, and code clarity, plus three sanity-check questions ("would clippy be clean?", "production-deployable?", "idiomatic?"). `v1.4.0/` was seeded by `cp -r v1.3.0 v1.4.0` (an earlier slip had left `v1.4.0/` as an empty directory, so the first pass nested everything inside `v1.4.0/v1.3.0/`; that nested copy was deleted afterwards once the audit edits had landed in the real `v1.4.0/` tree). Verified end-to-end with `cargo check` (clean), `cargo clippy -- -D warnings` (clean), `cargo test` (**19/19 pass**), `cargo fmt --check` (clean).

**Correctness fixes:**

- **Cancellation no longer relies on error-string matching.** `run_backup()` previously checked `err.to_string().contains("ABORTED")` to decide if a returned error was a user-initiated cancel; that's load-bearing on a string literal that anyone could change. Replaced with `let cancelled = token.is_cancelled();` snapshot taken before `state.unregister()`. The `CancellationToken` is now authoritative. Inner pipeline still returns `Err(anyhow!(CANCELLED_MSG))` on token-trip, but the message is no longer matched anywhere — `CANCELLED_MSG` is a single named constant just for grep-ability.
- **`path_contains` Windows-canonicalize asymmetry.** Surfaced by a new `path_contains_child_is_true` test: on Windows, `std::fs::canonicalize` prepends `\\?\` (and converts UNC paths to `\\?\UNC\…`) on existing paths but not on non-existing ones. If one input was canonicalisable and the other wasn't, the prefix check would never match. `normalize_for_compare()` now strips `\\?\UNC\` (rewriting back to `\\…`) and `\\?\` so both sides agree on surface form. Production already validated both inputs to exist before the call, so this only mattered when the source had been short-name-expanded (`YOSHIM~1`) — but the fix is the right invariant either way.
- **`Mutex` poisoning won't kill the scheduler.** `seen.lock().unwrap()` in `scheduler::tick()` panicked on poisoning. Replaced with `.unwrap_or_else(|poisoned| poisoned.into_inner())` — worst case is one stale "first observation" record; far better than the scheduler thread dying silently after an earlier panic.
- **`continueOnError` honoured for ETA division.** `maybe_emit()` had a manual `if speed > 0` guard before dividing; clippy flagged the latent div-by-zero path if `speed` ever got cast down to 0 from a small `f64`. Rewrote as `total_bytes.saturating_sub(copied_bytes).checked_div(speed)` — single expression, no branch, no panic possible.

**Idiom / clippy / fmt:**

- `&normalized[2..]` after `starts_with(r"\\")` → `if let Some(rest) = normalized.strip_prefix(r"\\")` — manual prefix-strip lint cleared.
- Two collapsible `if`s in `prune_destination` (the `is_dir()` no-op block and the `is_file && !keep && remove_file` chain) collapsed into single `&&`-chained guards.
- `split(|c: char| c == ',' || c == '\n')` → `split([',', '\n'])` in `glob::parse_patterns`.
- `setup_logging(&app.handle())` → `setup_logging(app.handle())` — `app.handle()` already returns `&AppHandle`, the `&` was an immediate-deref.
- The four-deep nested `if let`/`if` chain in the size+mtime skip clause flattened into a single `&&`-chained boolean using `.is_some_and(...)`.
- Unnecessary `dirs.iter().map(|(p, r)| (p.clone(), r.clone())).collect()` to produce a sortable vec replaced with `dirs.sort_by_key(...)` in place — `dirs` isn't reused after the dir-attrs loop, so re-cloning every `(PathBuf, String)` was pure waste. Made the `dirs` binding `mut`.

**Polish:**

- `main()`'s panic message changed from `"error while running tauri application"` to `"Tauri runtime failed to start (check logs in app_log_dir/driveby.log)"` so an end-user log dump tells operators where to look first.
- `info!("driveby 1.4 starting")` startup line, sidebar `Version 1.4` label (i18n EN + FR), `package.json` / `Cargo.toml` / `tauri.conf.json` all bumped to `1.4.0`.

**Tests added (12 new, 19 total, all green):**

- `path_contains_self_is_true`, `path_contains_child_is_true`, `path_contains_sibling_is_false`, `path_contains_prefix_lookalike_is_false` — security-relevant overlap rejection that previously had zero tests. The `_lookalike_` test specifically guards against `/a/b` falsely "containing" `/a/bb`. The `_child_` test created a `make_test_dir()` helper that `create_dir_all`s a temp subdir (Windows `temp_dir()` may return a short-name path that breaks canonicalize-vs-fallback comparison; in production both inputs exist so this is a test-fixture concern only).
- `parse_patterns_drops_blank_and_whitespace_only`, `empty_pattern_list_matches_nothing`, `double_star_crosses_directories`, `question_mark_is_single_non_slash`, `special_regex_chars_are_escaped` — fills out the glob parser's edge-case coverage.

**Items already passing without changes:**

- `unsafe` is confined to `apply_attrs` (Win32 `SetFileAttributesW` + `GetLastError`). Buffer is null-terminated, lives until end of function, BOOL return is checked, error logged.
- Module split (`glob`, `persist`, `restore`, `scheduler`, `backup`, `main`), `pub` boundaries (private internal helpers, narrow public surface for cross-module use), `tracing` exclusively in library code (no `println!`), `anyhow::Result` in library + `Result<_, String>` at the Tauri IPC boundary as the framework requires, `.context()` / `.with_context()` throughout.
- Crates: every dep is doing real work, none heavy or trivially substitutable. `windows-sys` features pinned to the two minimum subsystems actually called.

**Deferred for v1.5+:** `execute()` in `backup.rs` is ~350 lines of one function (preflight → walk → copy loop → prune → icon-verify → mirror-attrs → emit → optional verify). Splitting each phase into its own helper returning a typed `PhaseStats` accumulator would let each be tested in isolation. Refactor without functional change — explicitly out of scope for an audit pass.

## Key design decisions

- Tauri 2 desktop-only: dropped `[lib]` crate stanza (would be needed for mobile targets).
- Settings `confirmBeforeBackup`, `showNotifications`, `theme`, `accentColor`, `language`, `sidebarOpen`, `lastView` are UI-only. They live in `settings.json` but the Rust `Settings` struct only deserialises the fields the engine actually needs — extras are ignored on the way in and round-tripped untouched on the way out via the JS-side full-object `bridge.saveSettings`. (The earlier `#[serde(flatten)] _rest: Value` field was deleted in 1.2 — `Default` on `Value` is `Null`, not a map, which made `Settings::default()` produce an invalid flatten field.)
- Drag-and-drop of folders into forms removed (Tauri webview doesn't expose absolute paths for dropped files the way Electron did). "Choose…" button replaces it.
- Scheduler in 0.5–0.7 stayed in React — automatic runs required the app window to be open. From 0.7 onwards the scheduler runs in Rust as a background tokio task, so closing the window no longer skips a run.
- The 1.0 sync model: source ↔ destination is a true mirror (copy + skip-if-same + delete-orphans). No timestamped wrappers, no manifest. The restore module still ships for legacy `v0.7.0-beta` dated backups.
- Windows attribute preservation is essential — without `READONLY | HIDDEN | SYSTEM` propagation, custom folder icons defined via `desktop.ini` silently break in the destination.
- Cooperative cancellation flows through a `tokio_util::sync::CancellationToken` instead of error-message string-matching (1.4). The token is the single source of truth in `run_backup` for whether a returned `Err` represents a user-initiated cancel or a real failure.
