# Security audit against the OWASP DevSecOps Guideline

Date: 2026-09-26 · Version audited: 2.0.0 (`8fa5dc9`) ·
Reference: [OWASP DevSecOps Guideline](https://github.com/OWASP/DevSecOpsGuideline)

The audit split the codebase four ways: the Rust IPC surface and
persistence, the Rust copy/restore/snapshot engine, the webview and Tauri
configuration, and the CI/CD pipeline and supply chain. Every finding below
was checked against the code; the fixed ones have a regression test where
the behaviour is testable on Linux.

## Fixed

| Sev | Finding | Fix | OWASP |
|---|---|---|---|
| Critical | On Linux/macOS a `\` in a file name became `/` in the relative path, so a source or backup entry named `..\..\x` was written **outside the destination** by backup and by restore. | Only Windows translates `\` to `/` (`backup::rel_string`); elsewhere the name stays one component, so the file is backed up and restored under its own name, inside the destination. End-to-end tests fail without the fix. | 1-2-1, 2-1-1 |
| High | The updater signing key was reachable from the frontend build: Vite's `envPrefix: 'TAURI_'` matched `TAURI_SIGNING_PRIVATE_KEY`, and the release built the frontend next to the key. | `envPrefix` is `VITE_`/`TAURI_ENV_`; the release builds the frontend in its own step with no secrets and skips `beforeBuildCommand`. | 2-2-1-2, 2-3-6-3 |
| High | A notification's "open folder" button opened any path the webview supplied with the default handler — on Windows, ShellExecute runs an `.exe`, `.lnk` or `.bat`. | Only an existing absolute directory is opened. | 2-4-4, 2-1-2 |
| High | A folder dated in the future (`9999-12-31`, planted or left by a clock that ran ahead) became the day retention counted back from and **deleted every real version**. | Retention measures its window from the clock. | 2-1-1 |
| High | Release pipeline: actions pinned by tag, `contents: write` for every step including `npm ci`, credentials persisted, caches shared with CI, key in repository secrets. | SHA pins, per-job scopes, `persist-credentials: false`, no caches, `release` environment, `--ignore-scripts`. | 2-3-6-3, 2-2-1-4 |
| Medium | `start_backup` trusts the task the webview sends; pointed at the home folder a mirror would prune every document. | Refuse the home folder and its ancestors as a destination, in the run and in the preview. (See *Not fixed* for the rest.) | 2-1-2, 3-2 |
| Medium | `update_last_backup` wrote `[]` over a `tasks.json` it could not read (lock, permission, bad sector), losing every task. | Only a cleanly read document is written back. | 3-2 |
| Medium | Symlink planted at a scratch-file name on the destination was written through. | Scratch files are recreated with `create_new`. | 2-1-1 |
| Medium | Webview granted `core:default`, `opener:default` (open any http/https URL), `dialog:default`, `notification:default`, … | Narrowed to the ten permissions the frontend calls. | 2-1-2 |
| Medium | RUSTSEC-2026-0285 (rustls), RUSTSEC-2026-0194/0195 (quick-xml) in `Cargo.lock`. | Lockfile refreshed; cargo-deny gate added. | 2-3-2, 2-7-4 |
| Low | Size sums could wrap on files near 2^63 bytes and pass the room check. | Saturating arithmetic. | 1-2-1 |
| Low | Marker file read whole into memory whatever its size. | Capped at 64 KiB; non-files already fail the destination. | 1-2-1 |
| Low | CSP allowed an unused `asset:` protocol; no `object-src`/`base-uri`/`form-action`/`frame-src`; no `freezePrototype`. | Tightened. | 2-4-5 |
| Low | Logs (full paths) kept forever. | 14 days. | 2-7-2, 3-2 |
| Low | `translate()` read `$&` in a file name as a replace pattern. | Values inserted literally. | 1-2-1 |
| Low | Dead Tauri 1 updater keys (`active`, `dialog`) gave a false impression that the updater was off. | Removed. | 2-4-5 |

## Not fixed — recommended follow-ups

These need a design decision or platform testing this audit could not do.

1. **`start_backup` / `save_tasks` trust the webview (Medium).** Any script
   running in the webview can save a task with any destination and run it;
   the scheduler then runs it unattended. No XSS sink was found and the CSP
   is strict, so this needs a webview compromise first. Proper fix: take only
   a task id in `start_backup`, validate tasks in `save_tasks`, and only
   mirror-prune a destination that carries a Driveby marker or was empty on
   first use (needs a migration for existing destinations).
2. **Symlinks/junctions already inside a destination directory are followed
   by `create_dir_all` (Medium),** and the prune/remove walks re-read
   directories by path (TOCTOU). Fix with handle-relative, no-follow walks
   (`cap-std`/`rustix`, or `openat` with `O_NOFOLLOW`).
3. **Sparse or huge source files can make eviction delete every older
   version before refusing anyway (Medium).** Refuse when the need exceeds
   the volume's total size, and count sparse files by allocated blocks.
4. **Source files are opened by path after the walk (Low/Medium):** a file
   swapped for a symlink or a FIFO between walk and copy is followed, or
   blocks. Open with `O_NOFOLLOW | O_NONBLOCK` and check the handle.
5. **Backup and restore do not lock each other out (Low).**
6. **A future `lastBackup` stops a schedule silently (Low).** Treat
   timestamps well past "now" as invalid.
7. **The updater plugin is registered even in builds without a public key
   (Low):** a check still contacts GitHub and can show an "update available"
   that then fails to install (never installs unsigned code).
8. **`macOSPrivateApi` is enabled but unused (Low).** Remove it and the
   `macos-private-api` Cargo feature after a macOS build check.
9. **Exclude patterns that fail to compile are dropped silently (Low),**
   which removes prune protection for what they excluded.
10. **Formatting:** the tree has never been through `rustfmt`; a one-off
    `cargo fmt` commit (listed in `.git-blame-ignore-revs`) would let CI gate
    it.

## OWASP DevSecOps Guideline coverage

| Section | Status |
|---|---|
| 1 People (champions, training) | Process — not a code change |
| 2-1-1 Threat modeling | Threat model applied in this audit (webview as the untrusted side, tampered backup drive, attacker-influenced source tree) |
| 2-1-2 Secure design | Least-privilege capabilities, CSP, IPC input checks |
| 2-2-1-1 Pre-commit | `.pre-commit-config.yaml` (gitleaks, hygiene hooks) |
| 2-2-1-2 Secrets management | gitleaks in pre-commit and CI; key in a `release` environment; `envPrefix` fix. History scanned: no leaks |
| 2-2-1-3 Linting | ESLint (react, hooks, security) and clippy `-D warnings` in CI |
| 2-2-1-4 Repository hardening | SHA-pinned actions, least-privilege tokens, CODEOWNERS, Dependabot for actions; rulesets are a settings item |
| 2-3-1 SAST | CodeQL (JS, Rust, Actions), zizmor for workflows |
| 2-3-2 SCA | npm audit, cargo-deny, dependency review, Dependabot, daily advisory re-check |
| 2-3-3 Containers, 2-3-4 IaC | Not applicable — no images, no infrastructure code |
| 2-3-5 Security gates | Release preflight gate; CI checks to be made required |
| 2-3-6 Supply chain | CycloneDX SBOMs, SHA256SUMS, build-provenance attestations verified before publishing |
| 2-4-1 IAST, 2-4-2 DAST, 2-4-4 API | Not applicable as such — no server; the IPC surface was reviewed by hand |
| 2-4-3 Mobile | Not applicable — desktop only |
| 2-4-5 Misconfiguration | CSP, capabilities and updater config reviewed and tightened |
| 2-5-1 Release | Immutable published releases, advisory gate, verify-before-publish |
| 2-6 Deploy, 2-7-1 Cloud, 2-7-6 BAS | Not applicable — no servers; the updater's minisign check is the deploy gate |
| 2-7-2 Logging | Log retention bounded |
| 2-7-3 Pentest | Recommend a review of IPC and path handling before major releases |
| 2-7-4 Vulnerability management, 2-7-5 VDP | `SECURITY.md` (private reporting, targets), Dependabot, daily audit |
| 3-1-2 Policy as code | `deny.toml`, dependency-review licence allow-list |
| 3-2 Data protection | `tasks.json` data-loss fix, log retention, destination guard |
| 3-3, 3-4 Reporting, AI governance | Process — GitHub Security tab serves as the dashboard |
