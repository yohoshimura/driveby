# Driveby

A desktop backup app for keeping folders mirrored onto a local or external
drive. Built with Tauri 2 + Rust and React 18.

Point it at a folder, choose where the copy should live, and pick how often it
should run. Driveby keeps the destination matching the source and stays out of
the way the rest of the time.

## What it does

**Backup tasks.** Each task is a source folder, a destination, and a schedule —
manual, hourly, daily, weekly or monthly. Run one by hand at any time.

**Only what changed.** Files whose size and modification time already match are
skipped, so a repeat run over a large tree finishes in seconds. Up to eight
files copy at once; one at a time suits an older spinning disk better.

**A mirror, not snapshots.** The destination reflects the source as it is now.
Files you delete at the source are removed from the copy on the next run.
There is no version history to roll back through — see *Limits* below.

**Runs without you.** Closing the window can leave Driveby in the notification
area so scheduled backups still fire, and it can start with your session.

**Exclusions.** Skip files and folders by pattern: `*` for any characters,
`**` to cross folders, `?` for one character, and a leading `!` to bring
something back in.

**Verification.** Optionally read every copied file back and compare it against
a fingerprint taken while it was written, to catch corruption in transit.

**Restore.** Pick a past backup, choose where to put it, and watch it go — with
a progress bar and a stop button. Nothing at the destination is overwritten
until the replacement is safely written.

**History and statistics.** Every run is recorded with its size, file count,
duration and outcome, kept for as long as you choose.

On Windows it also handles the things that quietly break other copy tools:
paths beyond 260 characters, UNC shares, Hidden/System/ReadOnly attributes, and
custom folder icons.

Available in English and French, with light and dark themes.

## Install

Everything is on the
[latest release](https://github.com/yohoshimura/driveby/releases/latest) page.
The builds are unsigned, so Windows and macOS each raise a warning the first
time — the steps below say how to get past it.

**Windows.** Take the `-setup.exe`. It installs for the current user only, so
no administrator rights are needed, and it registers normally under
**Settings → Apps**, where it can be uninstalled like any other program.
SmartScreen shows a blue warning on first run: *More info*, then *Run anyway*.
An `.msi` is published alongside it for deployment tooling.

To run Driveby without installing it — from a USB stick, or on a machine you
cannot install software on — take the `-portable.exe` instead. It is the same
program, with nothing to uninstall afterwards. Settings, tasks and history go
to `%APPDATA%\com.driveby.app` either way, so a portable copy still remembers
its tasks, and still needs to be running for a schedule to fire.

**macOS.** Take the `_universal.dmg` — one build for both Apple Silicon and
Intel — and drag Driveby to Applications. macOS refuses the first launch
because the app is not notarised; open **System Settings → Privacy &
Security**, find the message naming Driveby, and click **Open Anyway**.

**Linux.** The `.AppImage` runs on any distribution: `chmod +x` it and go. For
a system install, take the `.deb` or the `.rpm` instead.

```bash
sudo apt install ./*_amd64.deb      # Debian, Ubuntu
sudo dnf install ./*.x86_64.rpm     # Fedora, RHEL
```

Driveby checks for updates on its own and installs them from the Settings
screen. A `.deb` or `.rpm` install is the exception: a packaged install is
updated through your package manager, not by the app.

## Limits

- **No versioned snapshots.** Restoring gives you the state of the last run, not
  a point in time you choose. This is a deliberate design choice, not an
  oversight — versioning changes the storage model entirely.
- Scheduled backups only run while Driveby is running, whether in a window or
  in the notification area.
- The tray menu is English-only.

## Building from source

```bash
npm install
npm run tauri dev      # run it
npm run tauri build    # produce installers
npm test               # frontend tests
cd src-tauri && cargo test
```

Layout: `src/` is the React frontend, `src-tauri/src/` is the Rust backend
(`backup.rs` is the copy engine, `restore.rs` the restore path, `scheduler.rs`
the timer). `scripts/` holds the version bump, the logo generator, and a
cleanup script for build output.

Release history is in [CHANGELOG.md](CHANGELOG.md).
