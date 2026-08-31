# Changelog

## 1.7.4

Every defect below was already in 1.7.3, and not one of them announced itself.
A backup that reports success while deleting the copy it was meant to protect,
an exclusion that excludes nothing, a Stop button that stops nothing for
another two minutes — each one looked, from the outside, exactly like Driveby
working. That is what this release is about.

**Two tasks writing to the same folder destroyed each other's backups.** Every
destination is mirrored against its own task's source, so whatever the other
task had just written was "not in my source" and went — file by file, counted
as cleaned, reported as a successful run. Then the other task ran and did the
same back. Both histories stayed green while neither backup ever survived a
cycle.

A destination that sits inside another task's destination is now refused at
the start of the run, and so is one that sits on top of another task's
*source* — the same hazard wearing a worse hat, since the prune would empty
the folder that task backs up from. The message names the task in the way, so
you can tell what to change. The form says it too, at the moment you pick the
folder, while it is still a choice.

**A settings or task file that would not parse was quietly replaced — and then
overwritten.** A truncated `tasks.json` — a power cut mid-write, a bad sector —
read as "no tasks". The list rendered empty, the next thing you touched saved
that empty list over the only copy, and the tasks were gone with nothing to
recover from. The same silence sat under `settings.json`, `history.json` and
`scheduler.json`.

A file that cannot be parsed is now moved aside as
`<name>.corrupt-<timestamp>` *before* the empty default is handed out, so the
original is still there when someone notices. A file that could not be *read*
is left exactly where it is: unreadable is not the same as known-bad.

**Exclusions written with a backslash excluded nothing.** `Photos\raw` — the
separator Windows puts in front of you everywhere else — compiled into a rule
that was accepted, listed, and could never match anything. The folder you
believed you had excluded was walked and copied on every run, for as long as
the pattern sat there. Both separators now mean the same thing.

**The scheduler could forget every clock it had, and stop scheduling because
of one bad task.** Both from the same cause: a `tasks.json` it could not read.
One hand-edited task made it abandon the whole list, and an antivirus holding
the file open for a moment did the same — and the empty result read as "the
user deleted every task", so the saved schedule clocks were wiped. Tasks that
had never run were pushed back a full interval; tasks that had failed forgot
they had, and retried at once. Those are the two things the saved clock exists
to prevent.

Tasks are now read one at a time, so a bad one costs only itself, and the
clocks are only cleaned up when the list can be believed. A `settings.json`
that will not parse is reported rather than silently swapped for the defaults
— which is how a scheduled run could ignore every exclusion you had set while
manual runs, whose settings come from the window in front of you, kept working
correctly.

**The unplugged-drive reminder could go quiet for good.** Its "already told
you" flag was cleared when *nothing* was missing, while the reminder fired
when *everything* was missing — and a task with two drives fell in the gap.
Lose both, plug one back in, lose both again, and it never spoke a second
time.

**The speed ceiling did not apply until the next backup.** It was read once,
when a run started. So the sequence the setting exists for — a backup is
saturating the disk, the machine is unusable, you open Settings and ask for
5 MB/s — did nothing at all until that run finished. It now takes effect as
soon as it is saved, mid-run.

The ceiling also delivered a little less than it promised: the copy loop reads
in 1 MiB chunks, every file ends on a short one, and that short chunk threw
away budget the full chunks had already waited for. It happened at the end of
every single file.

**Stop reaches the whole run now.** Two places ignored it. Cancelling a
preview left the source walk reading to the end of the tree — on a large one,
minutes of the dialog sitting there having acknowledged nothing; and a
superseded scan kept reading the disk at full cost alongside the one that had
replaced it. Stopping a backup left its last phase, the one that mirrors
folder attributes, running to completion — and until it returned, the task
counted as busy and no new run could start.

**A restore brought back the files, but not the folders.** It rebuilt the tree
out of files alone, so a folder that held something came back and a folder
that held nothing did not — the restored tree quietly had fewer directories
than the backup it came from. Folder attributes were not restored either,
which meant every folder with a custom icon came back with the default one,
the exact outcome the backup goes to some trouble to prevent. Directories are
now restored in their own right, deepest first, with their attributes.

### Behaviour changes worth knowing

- A `tasks.json`, `settings.json`, `history.json` or `scheduler.json` that
  cannot be parsed is renamed to `<name>.corrupt-<timestamp>` and left in the
  data folder. If a list ever comes back empty, look there before doing
  anything else.
- A task whose destination overlaps another task's destination or source is
  refused — when you pick the folder, and again at the start of every run. If
  you have such a pair today, both were destroying each other's backups; the
  refusal is the first you will hear of it.
- Exclude patterns are matched with `/` and `\` treated alike. A pattern that
  used a backslash to escape a `*` or a `?` never worked here, so nothing that
  matched before stops matching.
- The scheduler probes each distinct destination once per tick and probes them
  together. Fifteen tasks with two absent drives each used to spend sixty
  seconds of a sixty-second tick waiting, which made every interval schedule
  drift later.

## 1.7.3

The Mac build has been published since 1.7.0 and had never been tested. Three
defects were waiting there, and all three are the same oversight: the code
asked "is this Windows?" when the question it meant was "does this filesystem
ignore case?".

**A backup on a Mac could delete the folder you excluded.** APFS is
case-insensitive, exactly as NTFS is. Rename a folder's case at the source —
`raw` to `RAW` — and the destination keeps the old spelling, because a
case-preserving filesystem writes through the entry it already has. Five
separate places decided for themselves whether to compare spellings case-
insensitively, and every one of them answered "no" on a Mac: the exclusion
matched the source's casing, the prune pass saw the destination's, the
protection was not recognised, and the subtree went. This is the defect 1.7.0
fixed on Windows, still live on the platform nobody had run the tests on.

They now read one answer, so the five cannot drift apart again.

**A restore could empty every file it was meant to bring back.** The check
that refuses a source and destination nested in one another compared paths
case-sensitively off Windows. On a Mac `~/Photos` and `~/photos` are one
directory, so the two were accepted as unrelated — and restoring a backup
onto itself opens each file for writing before reading it.

**Directory re-casing works on a Mac.** The pass that gives a re-cased folder
its source spelling back never ran there. It does now, resolving a directory's
real name by reading its parent — memoized per parent, so a folder holding ten
thousand subdirectories is read once rather than ten thousand times.

**Driveby leaves the Dock when it goes to the menu bar.** Closing to the tray
left a Dock tile behind for a window that no longer existed, and starting at
login put one there for an app that deliberately has no window yet. The tile
now comes and goes with the window.

**The name is spelled Driveby.** A capital D wherever it is shown — window
title, tray tooltip, sidebar, every string in both languages, the installers.
Settings, tasks and history stay exactly where they are.

### Behaviour changes worth knowing

- The macOS app bundle and the Windows installers are named `Driveby`. On both
  systems that is the same path as before, since neither filesystem
  distinguishes the case — the subject of this whole release. Linux packages
  genuinely change name, so the README's install commands no longer spell it.
- CI now builds and tests on macOS alongside Windows and Linux. It caught a
  defect in this release's own first draft.

## 1.7.2

Five features, all of them about the same thing: knowing what a backup is about to do, and where it is going.

**A task can back up to several destinations.** One source, as many drives as you like — an internal disk, an external one, one that lives at a friend's house. Keeping a folder on three drives used to mean three tasks: three schedules to keep in step, three unrelated history rows, and no way to notice that the copy on the drive in the drawer was six weeks behind the other two.

The destinations are written one after another, from a single snapshot of the source, so the three copies are made from the same tree rather than from three trees minutes apart. A drive that is not connected no longer costs the others their backup: the ones that are there are written, the run is reported as *partial*, and the schedule clock does not advance — so a drive left unplugged stays visible instead of hiding behind a fresh timestamp.

Destinations nested inside one another are refused before anything is written. Each destination is mirrored against the source, which means one sitting inside another would be deleted by its host on the very first run — a backup erasing a backup.

**Confirming a backup now says what it will do.** The old dialog showed the source and the destination, two paths you picked yourself. It now counts the work first: how many files are new, how many changed, and how many are about to be **deleted** from the copy — the one thing a mirror backup does that cannot be undone. Deletions are coloured, and the confirm button turns destructive when there are any.

The count is worked out by the same code that performs the run, so the dialog cannot promise one thing and the backup do another. On a large tree the scan takes a moment; the dialog opens straight away and can be cancelled while it works. Scheduled runs never wait for it.

**A speed ceiling.** Settings takes a maximum in MB/s, empty meaning no limit. It is shared by every backup running at once — the point being that the machine stays usable while one runs, and three tasks firing together must not add up to three times the ceiling. Stopping a run still stops it immediately, even at a low ceiling.

**Schedules can follow the clock.** Alongside hourly, daily, weekly and monthly — which have always meant "this long since the last one" — a schedule can now name days and a time: Monday and Thursday at 22:00, in your own timezone. The form shows the next run as you set it. A missed occurrence is caught up rather than skipped, so closing the app at 22:00 and opening it at 23:10 still runs the backup; a run that fails waits for the next occurrence rather than retrying every minute.

**An unplugged drive is a reminder, not an error.** A scheduled task whose destinations are all missing used to walk the source, fail, and file a red history row — once per interval, for ever. It now does not start at all: you are told once, and the task card shows which destination is missing while it stays that way. Plugging the drive back in does not set a backup going while you are still handling the disk; it goes at the next scheduled time.

### Behaviour changes worth knowing

- `tasks.json` is rewritten once on first launch to hold a list of destinations instead of a single one. An older version of Driveby can still read the result.
- History has one row per run, with a line per destination underneath carrying its own state, Restore and Reveal. Sizes and file counts are totals across destinations: three copies of a 4 GB source really did move 12 GB.
- A partial run counts with the errors in Statistics — something you asked for did not get backed up.
- The chart of backed-up data no longer paints a task in the error red.

## 1.7.0

A correctness release. A review of the whole codebase turned up sixteen defects; three of them could destroy data that had already been backed up, and all three were the same mistake made in three places.

**A backup that cannot be read no longer destroys the copy it was meant to replace.** The retry loop deleted the destination file *before* it had opened the source. A file locked by another process — a running VM holding its own disk image is the everyday case — meant the previous copy was gone and nothing replaced it, while the run carried on and reported a single failure. Copies now stream into a scratch file beside the destination and swap in with an atomic rename, so nothing that was already safe is at risk until the replacement is on disk. The same change means cancelling a backup no longer removes files from the backup.

**Cancelling a restore no longer deletes the file you already had.** The destination was opened with `File::create`, which truncates it, before the first byte was read. Cancelling then removed the emptied file — leaving neither your version nor the backup's. A bad sector on the backup drive, the exact situation a restore exists for, did the same. Restore now uses the same scratch-and-rename path.

**`history.json` can no longer be corrupted by two runs finishing at once.** Every writer shared one fixed scratch filename, so a second write truncated the first mid-flight and the file ended up as interleaved JSON. On the next launch it failed to parse and the whole history was silently replaced with an empty list. Scratch names are now unique per write, and history writes are serialised the way task writes already were.

**Stopping a backup holds the task until the run has actually finished.** Cancelling released the task's slot immediately, while the run was still draining its in-flight copies. Starting again straight away put two runs on the same destination, deleting each other's files under a run that reported success. A late-finishing run could also evict a newer one's cancellation token, leaving it impossible to stop.

**A cancellation that lands late is reported as a cancellation.** Stopping a run after its last checkpoint let it return success, record a successful entry, and stamp `lastBackup` — resetting the schedule clock on a run the user had stopped.

**Excluding a folder no longer deletes it from the backup.** Exclusions were matched case-sensitively against the destination's spelling while being keyed by the source's. Renaming a folder's case and then excluding it made the prune pass fail to see the protection and remove the whole subtree — the exact opposite of what "exclude" means.

**Fixes across the interface.** Holding Enter no longer confirms a delete before the dialog can be read. The update button now has a real installing state, instead of reverting to an enabled "Check for updates" mid-download and inviting a second install. Escape while a confirmation is open no longer also closes the task form behind it and discards what was typed. Settings saved at launch no longer write the defaults over what was just loaded from disk. The restore confirmation now names the backup it is about to write, not only the destination.

**A release can no longer ship the wrong version.** Nothing checked the tag against the manifests, so tagging without running `npm run bump-version` produced a green release whose `latest.json` still pointed at the previous version — every client reporting "up to date" forever. CI now fails on the mismatch. A manual workflow run can no longer draft a release tagged with a branch name, and the platform jobs no longer race each other writing `latest.json`.

**A new logo, and a launch animation.** The old icon had a white halo baked into its pixels from being flattened onto a white background — invisible on white, an obvious outline on a dark taskbar. It has been rebuilt as vector geometry, which also made the launch animation possible: the drive appears, the arrow drops into it, the status lights come up.

**Shorter help bubbles** in Settings, and the tray menu now reads *Open* and *Quit*.

### Behaviour changes worth knowing

- Copies now pass through a `.driveby-tmp` file beside each destination file. A run killed mid-copy can leave one behind; the next run's prune sweeps it.
- Exclusion patterns match case-insensitively on Windows.
- Cancelling a backup keeps the task busy until the run has finished stopping, so a second run cannot start in that window.
- Stopping a run no longer advances `lastBackup`, so the schedule is not reset by a cancellation.

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
