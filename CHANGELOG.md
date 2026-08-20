# Changelog

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
