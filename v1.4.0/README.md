# driveby 1.4

A local-drive backup app with a macOS-style sidebar UI — **Tauri 2 + Rust** backend, **React 18 + Vite** frontend.

1.4 is a Rust **audit pass** on top of 1.3 — no UI surface change, no new feature, just a sweep through the backend against a correctness / idiom / safety / test-coverage checklist. Verified clean on `cargo check`, `cargo clippy -- -D warnings`, `cargo fmt --check`, and **19/19 unit tests pass**.

> **Note on numbering:** what was previously branded `v2.x` was renumbered to `v1.x` (the actual 1.0+ shipping line, post-Tauri-rebrand). The pre-Tauri Electron snapshots that used to be `v1.x-beta` are now `v0.x-beta`. This README refers to features by the new numbers; `conversation.md` keeps the original session-time labels for historical accuracy.

## What's fixed / improved in 1.4

| Area | Change |
|------|--------|
| **Cancellation is type-safe** | `run_backup` previously discriminated cancelled-vs-failed by `err.to_string().contains("ABORTED")`. Now reads from the `CancellationToken` directly via `token.is_cancelled()` (snapshot before `unregister`). The inner pipeline still returns `Err(anyhow!(CANCELLED_MSG))` on token-trip, but no caller string-matches the message anymore. |
| **`path_contains` Windows-canonicalize fix** | Surfaced by a new test: `std::fs::canonicalize` prepends `\\?\` (and rewrites UNC to `\\?\UNC\…`) on existing paths but not on non-existing ones. If one input was canonicalisable and the other wasn't, the prefix-based containment check would never match. `normalize_for_compare()` now strips both forms so both sides agree on surface representation. |
| **Scheduler survives Mutex poisoning** | `seen.lock().unwrap()` would panic the scheduler thread on poisoning. Now `.unwrap_or_else(|p| p.into_inner())` — worst case is one stale "first observation" record, harmless; the scheduler keeps running. |
| **`maybe_emit` ETA division** | Replaced manual `if speed > 0 { Some(.../speed) }` guard with `.checked_div(speed)` — single expression, no latent div-by-zero panic. |
| **clippy / fmt clean** | Manual prefix-strip → `strip_prefix`. Collapsible `if`s collapsed. `split(\|c: char\| ...)` → `split([',', '\n'])`. `&app.handle()` → `app.handle()`. Four-deep nested skip clause flattened with `.is_some_and(...)`. Unnecessary `dirs.iter().map(\|x\| x.clone()).collect()` replaced with `dirs.sort_by_key(...)` in place. All driven by `cargo clippy -- -D warnings` exit code 0. |
| **`main()` panic message** | `"error while running tauri application"` → `"Tauri runtime failed to start (check logs in app_log_dir/driveby.log)"` — points operators at the log file when the Tauri runtime itself fails to start. |
| **Tests added (12 new, 19 total)** | `path_contains_self_is_true`, `_child_is_true`, `_sibling_is_false`, `_prefix_lookalike_is_false` (the security-relevant overlap rejection had zero coverage before). `glob`: `parse_patterns_drops_blank_and_whitespace_only`, `empty_pattern_list_matches_nothing`, `double_star_crosses_directories`, `question_mark_is_single_non_slash`, `special_regex_chars_are_escaped`. |

## What's retained from 1.3

EN / FR language switcher (every user-visible string flows through `useT()` / the in-tree `i18n.js`), the language section in Settings, every fix from 1.2 (atomic concurrent-run guard, exclude-aware prune, source/dest overlap check, 2-second mtime tolerance, restore durability, shared `tasks.json` lock, `continueOnError` on `create_dir_all`, final-failure cleanup in `copy_with_retries`, no-thundering-herd scheduler, single-slice pies, memoised key bindings, no dangling confirm promises, destination root keeps its icon, per-subfolder icons round-trip, dead-code prune), the post-1.3 folder-icon hash-verification phase, the red external-drive app icon.

## Prerequisites / Running

```bash
cd v1.4.0
npm install
npm run tauri dev
npm run tauri build
```

## Open items for 1.5+

- **`execute()` decomposition.** ~350 lines of one function (preflight → walk → copy loop → prune → icon-verify → mirror-attrs → emit → optional verify). Splitting each phase into its own helper returning a typed `PhaseStats` accumulator would let each be tested in isolation — pure refactor, no functional change.
- Locale-aware date/number formatting via `Intl.DateTimeFormat` / `Intl.NumberFormat` driven by the active language.
- Parallel copies, platform-native fast copy (`clonefile` / `copy_file_range` / `CopyFile2`), streaming hash-during-copy, per-run snapshots with restore-from-snapshot, history virtualization.
