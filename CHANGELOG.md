# Changelog

## 1.6.1

**The task card is quieter while a backup runs.** The running card carried a detail line under the progress bar — phase, file counts, throughput, ETA. It is gone. The bar is the whole running-state affordance now, and it takes an accessible name of its own since no text sits beside it any more. `backup.phase.*` and `formatSpeed` had no other consumer and went with it. The backend still emits phase, counts, speed and ETA — only the display changed.

**History is kept by age, not by count.** `historyLimit` (100 / 500 / 1000 / 5000 / All entries) becomes `historyRetention` — 1 day, 1 week, 1 month, 1 year or All, defaulting to a month — because "how far back can I look" is the question the setting actually answers. A month means a *calendar* month, clamping the day of the month the way the Rust scheduler's `checked_add_months` does: 31 March minus a month is 28 February, not 3 March. An entry whose timestamp is missing or unparseable is kept — we do not delete what we cannot date.

A hard cap of 5000 entries now sits underneath every choice, `All` included. The old count *was* the only thing bounding `history.json`, and an age rule on its own would let an hourly task write 8 760 rows a year. The cap is deliberately not surfaced in the UI.

There is no migration: a count and an age have no meaningful mapping between them, so the old `historyLimit` key is dropped from `settings.json` on load and every install starts at the one-month default.

**`npm run bump-version` writes `package-lock.json` too.** 1.6.0 shipped with a lockfile still claiming 1.5.0, because the script only knew about `package.json` and `Cargo.toml`.

### Behaviour changes worth knowing

- History now expires by age. The first launch after updating drops runs older than a month, whatever the previous count-based setting kept. Backup folders are never affected.
- `All` no longer means unlimited — it means *no date limit*. The 5000-entry cap still applies.

## 1.6.0

**Restore parity.** Restore can be cancelled (`cancel_restore`, byte-granular, partial file removed), refuses a second concurrent run, and shows live progress with a stop button. Its progress events are throttled to 100 ms instead of one per file.

**Performance.** Files copy in parallel (`parallelCopies`, default 4, `1` reproduces the old sequential behaviour). Copies are fingerprinted with xxh3 as they are written, so `verify` re-reads only the destination instead of both sides. History is capped (`historyLimit`, default 1000) instead of growing without bound. Live progress moved to its own React context, ending the whole-tree re-render on every progress tick.

**Correctness.** A folder re-cased at source is now re-spelled at the destination (1.5 only stopped it being deleted). Schedules support hourly; "monthly" is a calendar month rather than 30 fixed days; the retry clock survives an app restart via `scheduler.json`. Two toasts in quick succession no longer cut each other short. A render error shows a recovery screen instead of a blank window.

**Locale-aware formatting.** Dates, byte sizes, thousands separators, chart day labels and plurals follow the app's language rather than the OS locale, via `Intl.DateTimeFormat`, `Intl.NumberFormat` and `Intl.PluralRules`. French uses `Ko`/`Mo`/`Go` and a decimal comma.

**Background operation.** Optional (default off) close-to-tray and start-at-login, so scheduled backups run with the window closed. A tray icon provides the way back.

**Distribution.** GitHub Actions run the test suites on Windows and Linux and build Windows/macOS/Linux bundles on a tag. Auto-update is wired end to end and activates once signing keys are configured — see the README. The version now lives in two manifests instead of five; `npm run bump-version <x.y.z>` updates both.

### Behaviour changes worth knowing

- `verify` now checks files copied by the current run. Files skipped as unchanged were verified by the run that copied them.
- A scheduled task that has never run can fire shortly after an app restart once its interval has elapsed — the clock is persistent now, rather than restarting with the process. This is the intended fix.
- History beyond the cap is dropped on save. Backup folders are never affected.

## 1.5.0

Correctness sweep: five silent data-loss paths in backup and restore, Windows READONLY and folder-icon handling, scheduler retry spacing. See the README for the full table.
