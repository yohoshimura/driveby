# Multi-source Backup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one task back up several source folders into one destination, each source into its own subfolder.

**Architecture:** `FileEntry.rel` is the only thing that decides where a file lands at the destination. So each source is walked separately by the unchanged `walk()`, every `rel` in the result is prefixed with that source's destination subfolder, and the results are merged into the single `WalkResult` the rest of the pipeline already expects. Nothing downstream of the walk changes: one `KeepSet`, one `ProtectedSet`, one prune over the whole destination, one progress total.

**Tech Stack:** Rust (Tauri 2, tokio, serde, anyhow), React 18 + Vite, vitest, `cargo test`.

**Spec:** `docs/superpowers/specs/2026-09-02-multi-source-backup-design.md`

## Global Constraints

- **Never run `cargo fmt`.** These sources are not rustfmt-clean and CI does not check formatting.
- Commit messages: imperative, sentence case, **no** `feat:`/`fix:` prefix, naming the effect rather than the mechanism — *"Restore the folders, not only the files in them"*.
- Every commit ends green: `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, and `npm test`.
- Rust tests live in the existing `#[cfg(test)] mod tests` of the file they cover, using its `scratch(name)` helper, with a doc comment stating *what breaks* without them.
- Both the `en` and `fr` blocks of `src/lib/i18n.js` get every new key. There are exactly two locales (`SUPPORTED_LANGUAGES = ['en', 'fr']`).
- The Rust `Task::sources()` and the JS `taskSources()` read the same `tasks.json` and **must agree exactly** — the scheduler can tick before the frontend has migrated the file, and a downgrade rewrites the old shape.
- Baseline before starting: 123 Rust tests, 84 JS tests, all passing at `7fa19e2`.

---

## File Structure

| File | Responsibility after this change |
|---|---|
| `src-tauri/src/backup.rs` | `Source`, `Task::sources()`, `preflight_sources`, `walk_all`, plural overlap guards, `execute_all` wiring |
| `src-tauri/src/preview.rs` | Same walk and the same source list, so the preview describes the layout the run produces |
| `src-tauri/src/fsutil.rs` | Unchanged. `reject_overlap`, `path_contains`, `CASE_INSENSITIVE_FS` are reused as they are |
| `src/lib/task.js` | `taskSources`, folder-name validation, `migrateTasks` and `findForeignOverlap` extended |
| `src/lib/i18n.js` | New form keys, `en` and `fr` |
| `src/components/NewTaskForm.jsx` | The source list UI and its refusals |
| `src/components/TaskCard.jsx`, `src/components/charts/TaskList.jsx` | Display N sources instead of `task.source` |
| `src/context/AppContext.jsx:333` | The draft-validity check reads sources, not `source` |

`walk()` itself stays single-source and gains nothing. All multi-source logic lives in `walk_all`.

---

## Task 1: The data model — `Source` and `Task::sources()`

**Files:**
- Modify: `src-tauri/src/backup.rs:25-76` (the `Task` struct and its `impl`)
- Test: `src-tauri/src/backup.rs`, test module (alongside `task_destinations_normalises_both_shapes` at :2544)

**Interfaces:**
- Produces: `pub struct Source { pub path: String, pub folder: String }` (derives `Serialize, Deserialize, Clone, Debug, PartialEq`); `Task::sources(&self) -> Vec<Source>`; field `Task.source: Option<String>` (was `String`), field `Task.sources: Option<Vec<Source>>`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The legacy shape every tasks.json written before this change holds:
    /// one `source` string. It has to keep reading, because the scheduler
    /// deserialises this struct and can tick before the frontend migration
    /// has run — and a user who downgrades writes the old shape back.
    #[test]
    fn task_sources_normalises_both_shapes() {
        let legacy: Task = serde_json::from_value(serde_json::json!({
            "id": "1", "name": "t", "source": "C:/Photos"
        }))
        .unwrap();
        assert_eq!(
            legacy.sources(),
            vec![Source { path: "C:/Photos".into(), folder: "Photos".into() }]
        );

        let plural: Task = serde_json::from_value(serde_json::json!({
            "id": "1", "name": "t",
            "source": "C:/Ignored",
            "sources": [
                { "path": "C:/Photos", "folder": "Photos" },
                { "path": "C:/Work/Photos", "folder": "Photos-Work" }
            ]
        }))
        .unwrap();
        assert_eq!(plural.sources().len(), 2, "the plural field wins outright");
        assert_eq!(plural.sources()[1].folder, "Photos-Work");
    }

    /// Blanks and exact repeats go, the way `destinations()` drops them.
    /// Two spellings of one folder are left in on purpose: deciding they are
    /// the same folder means asking the filesystem, which is the overlap
    /// guard's job and it runs anyway.
    #[test]
    fn task_sources_drops_blanks_and_exact_repeats() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "1", "name": "t",
            "sources": [
                { "path": "C:/Photos", "folder": "Photos" },
                { "path": "   ", "folder": "Blank" },
                { "path": "C:/Photos", "folder": "Photos" }
            ]
        }))
        .unwrap();
        assert_eq!(task.sources().len(), 1);
    }

    /// A task carrying neither field has no sources at all, rather than one
    /// source whose path is the empty string — which would read as the
    /// current directory and back up something nobody asked for.
    #[test]
    fn a_task_with_no_source_field_has_no_sources() {
        let task: Task =
            serde_json::from_value(serde_json::json!({ "id": "1", "name": "t" })).unwrap();
        assert!(task.sources().is_empty());
    }

    /// A legacy path with a trailing separator still yields a usable folder
    /// name: `file_name()` on "C:/Photos/" answers "Photos", but on a bare
    /// root like "D:/" it answers nothing, and a source with no name of its
    /// own cannot be given a subfolder automatically.
    #[test]
    fn a_legacy_source_folder_comes_from_the_paths_own_name() {
        let with_slash: Task = serde_json::from_value(serde_json::json!({
            "id": "1", "name": "t", "source": "C:/Photos/"
        }))
        .unwrap();
        assert_eq!(with_slash.sources()[0].folder, "Photos");

        let bare_root: Task = serde_json::from_value(serde_json::json!({
            "id": "1", "name": "t", "source": "D:/"
        }))
        .unwrap();
        assert!(bare_root.sources().is_empty(), "a root has no name to become a folder");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml task_sources`
Expected: FAIL — `Source` is not defined, `Task::sources` does not exist.

- [ ] **Step 3: Implement**

Replace `pub source: String,` in the `Task` struct (`backup.rs:28`) with the legacy/plural pair, mirroring the `destination`/`destinations` comment already directly below it:

```rust
    /// The shape every tasks.json written before multi-source holds: exactly
    /// one source. Kept readable for the same reason `destination` is — the
    /// scheduler deserialises this struct and can tick before the frontend
    /// migration has run, and a user who downgrades writes it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<Source>>,
```

Add the struct above `Task`:

```rust
/// One source folder, and the destination subfolder it writes into.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub path: String,
    /// A folder *name*, not a path: no separators, no `.` or `..`. Validated
    /// by `preflight_sources` before anything is written.
    pub folder: String,
}
```

Add to `impl Task`, directly above `destinations()`:

```rust
    /// Every source this task reads, in the order the user listed them,
    /// blanks dropped and exact repeats collapsed.
    ///
    /// Only exact repeats, for the same reason `destinations()` gives: two
    /// spellings of one folder are a filesystem question, and
    /// `reject_destination_overlaps` has to ask it anyway.
    pub fn sources(&self) -> Vec<Source> {
        let listed = match &self.sources {
            Some(list) if !list.is_empty() => list.clone(),
            // A legacy task's subfolder is the source path's own name. A path
            // with no final component — a bare drive root — cannot supply one,
            // and is dropped rather than given an invented name.
            _ => self
                .source
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|s| {
                    Path::new(s)
                        .file_name()
                        .map(|n| Source {
                            path: s.to_string(),
                            folder: n.to_string_lossy().to_string(),
                        })
                })
                .into_iter()
                .collect(),
        };
        let mut seen = HashSet::new();
        listed
            .into_iter()
            .map(|s| Source { path: s.path.trim().to_string(), folder: s.folder.trim().to_string() })
            .filter(|s| !s.path.is_empty() && !s.folder.is_empty())
            .filter(|s| seen.insert(s.path.clone()))
            .collect()
    }
```

- [ ] **Step 4: Fix the two remaining `task.source` readers so the crate compiles**

`backup.rs:654` (`preflight_source`) and `backup.rs:718` (`reject_foreign_overlaps`) both do `PathBuf::from(&task.source)` / `PathBuf::from(&other.source)`, which no longer compiles against an `Option<String>`. Make them read through the accessor for now; Task 4 reworks both properly:

- `:654` → `let source = PathBuf::from(&task.sources().first().map(|s| s.path.clone()).unwrap_or_default());`
- `:718` → iterate: `for their in other.sources() { let their_source = PathBuf::from(&their.path); … }`, keeping the existing message and moving the existing `if path_contains(mine, &their_source)` block inside the new loop.

Every test that builds a `Task` literal also needs `source: Some("…".into()), sources: None`. Search for `Task {` in the test module.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS, 127 tests (123 + 4).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/backup.rs
git commit -m "Let a task name more than one source"
```

---

## Task 2: `walk_all` — walk each source, prefix, merge

**Files:**
- Modify: `src-tauri/src/backup.rs`, new function directly below `walk()`
- Test: `src-tauri/src/backup.rs`, test module

**Interfaces:**
- Consumes: `Source` and `Task::sources()` from Task 1; `walk(root, patterns, token) -> Result<WalkResult>` unchanged.
- Produces: `async fn walk_all(sources: &[Source], patterns: &glob::PatternSet, token: &CancellationToken) -> Result<WalkResult>`.

Not wired into the pipeline yet — Task 3 does that. This task is testable on its own.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The feature's central property: two sources land in two subtrees and
    /// never interleave, and the absolute path each entry was read from is
    /// untouched — only where it is written changes.
    #[tokio::test]
    async fn walk_all_prefixes_each_source_with_its_folder() {
        let root = scratch("walk-all-two");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("a/one.txt"), b"1").unwrap();
        std::fs::write(root.join("b/two.txt"), b"22").unwrap();

        let sources = vec![
            Source { path: root.join("a").to_string_lossy().into(), folder: "Alpha".into() },
            Source { path: root.join("b").to_string_lossy().into(), folder: "Beta".into() },
        ];
        let walked = walk_all(&sources, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();

        let mut rels: Vec<&str> = walked.files.iter().map(|f| f.rel.as_str()).collect();
        rels.sort();
        assert_eq!(rels, vec!["Alpha/one.txt", "Beta/two.txt"]);
        assert_eq!(walked.total_bytes, 3, "sizes are summed across sources");
        assert!(
            walked.files.iter().all(|f| f.path.is_absolute()),
            "the read side keeps the real source path"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A source with no files still has to materialise its folder, or it
    /// vanishes from the destination without anything saying so.
    #[tokio::test]
    async fn walk_all_lists_every_source_folder_even_an_empty_one() {
        let root = scratch("walk-all-empty");
        std::fs::create_dir_all(root.join("empty")).unwrap();

        let sources = vec![Source {
            path: root.join("empty").to_string_lossy().into(),
            folder: "Empty".into(),
        }];
        let walked = walk_all(&sources, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();

        assert!(walked.files.is_empty());
        assert!(
            walked.dirs.iter().any(|(_, rel)| rel == "Empty"),
            "the folder itself must be in dirs, got {:?}",
            walked.dirs
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The one that matters. A source on an unplugged drive contributes no
    /// files, so prune would find its whole subfolder missing from the keep
    /// set and delete the only copy of it that exists. Recording the folder
    /// as unreadable is what makes ProtectedSet::covers skip that subtree.
    #[tokio::test]
    async fn a_source_that_cannot_be_read_protects_its_own_subfolder() {
        let root = scratch("walk-all-missing");
        std::fs::create_dir_all(root.join("here")).unwrap();
        std::fs::write(root.join("here/kept.txt"), b"k").unwrap();

        let sources = vec![
            Source { path: root.join("here").to_string_lossy().into(), folder: "Here".into() },
            Source { path: root.join("gone").to_string_lossy().into(), folder: "Gone".into() },
        ];
        let walked = walk_all(&sources, &glob::PatternSet::new(&[]), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(walked.files.len(), 1, "the reachable source still backs up");
        assert!(walked.unreadable.contains("Gone"));
        let protected = ProtectedSet::new(&walked, &glob::PatternSet::new(&[]));
        assert!(protected.covers("Gone"), "prune must walk around the missing source");
        assert!(!protected.covers("Here"));
        assert!(
            !protected.source_root_unreadable(),
            "one source failing must not stop prune for the others"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// When nothing at all could be read, failing is the only honest answer.
    /// Returning an empty walk would have the run report success, stamp
    /// lastBackup, and leave the scheduler believing the task ran — while a
    /// single missing source fails the run outright today.
    #[tokio::test]
    async fn walk_all_fails_when_no_source_can_be_read() {
        let root = scratch("walk-all-all-missing");
        let sources = vec![
            Source { path: root.join("gone-a").to_string_lossy().into(), folder: "A".into() },
            Source { path: root.join("gone-b").to_string_lossy().into(), folder: "B".into() },
        ];
        assert!(
            walk_all(&sources, &glob::PatternSet::new(&[]), &CancellationToken::new())
                .await
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Exclusions are recorded as relative paths and consulted by prune, so
    /// they have to move into the subfolder with everything else — otherwise
    /// prune looks up "node_modules" while the destination holds
    /// "Alpha/node_modules" and deletes what the user asked to keep.
    #[tokio::test]
    async fn walk_all_prefixes_the_excluded_set_too() {
        let root = scratch("walk-all-excluded");
        std::fs::create_dir_all(root.join("a/skipme")).unwrap();
        std::fs::write(root.join("a/skipme/x.txt"), b"x").unwrap();
        std::fs::write(root.join("a/keep.txt"), b"k").unwrap();

        let sources = vec![Source {
            path: root.join("a").to_string_lossy().into(),
            folder: "Alpha".into(),
        }];
        let patterns = glob::PatternSet::new(&["skipme".to_string()]);
        let walked = walk_all(&sources, &patterns, &CancellationToken::new())
            .await
            .unwrap();

        assert!(
            walked.excluded.iter().all(|e| e.starts_with("Alpha/")),
            "got {:?}",
            walked.excluded
        );
        let _ = std::fs::remove_dir_all(&root);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml walk_all`
Expected: FAIL — `walk_all` not found.

- [ ] **Step 3: Implement**

Add directly below `walk()`:

```rust
/// Walk every source and merge the results into the single `WalkResult` the
/// rest of the pipeline already expects.
///
/// `FileEntry.rel` is the only thing that decides where a file lands —
/// `copy_one` writes `target.join(&file.rel)` — so putting each source in its
/// own destination subfolder is entirely a matter of prefixing `rel`. `walk`
/// itself stays single-source and knows nothing about any of this.
///
/// A source that cannot be read does not fail the run: it contributes no
/// files and records its folder as unreadable, which is what makes
/// `ProtectedSet::covers` have prune walk around that subtree instead of
/// deleting the only copy of a folder whose drive is merely unplugged.
/// Failing every source *is* fatal — an empty walk would otherwise report
/// success and stamp `lastBackup` over a run that did nothing.
async fn walk_all(
    sources: &[Source],
    patterns: &glob::PatternSet,
    token: &CancellationToken,
) -> Result<WalkResult> {
    let mut merged = WalkResult {
        files: Vec::new(),
        dirs: Vec::new(),
        total_bytes: 0,
        skipped: 0,
        excluded: HashSet::new(),
        unreadable: HashSet::new(),
    };
    let mut last_error: Option<anyhow::Error> = None;
    let mut walked_any = false;

    for source in sources {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let root = PathBuf::from(&source.path);
        let prefix = source.folder.as_str();

        // The folder itself, so a source with no files still materialises at
        // the destination — `mirror_dir_attrs_phase` is what creates it.
        merged.dirs.push((root.clone(), prefix.to_string()));

        let walked = match walk(&root, patterns, token).await {
            Ok(w) => w,
            Err(e) if token.is_cancelled() => return Err(e),
            Err(e) => {
                warn!(source = %root.display(), "source could not be read: {}", e);
                merged.unreadable.insert(prefix.to_string());
                merged.skipped += 1;
                last_error = Some(e);
                continue;
            }
        };
        walked_any = true;

        let join = |rel: &str| {
            if rel.is_empty() {
                prefix.to_string()
            } else {
                format!("{}/{}", prefix, rel)
            }
        };
        merged.total_bytes += walked.total_bytes;
        merged.skipped += walked.skipped;
        merged
            .files
            .extend(walked.files.into_iter().map(|f| FileEntry { rel: join(&f.rel), ..f }));
        merged
            .dirs
            .extend(walked.dirs.into_iter().map(|(p, rel)| (p, join(&rel))));
        merged.excluded.extend(walked.excluded.iter().map(|r| join(r)));
        merged.unreadable.extend(walked.unreadable.iter().map(|r| join(r)));
    }

    if !walked_any {
        return Err(match last_error {
            Some(e) => e,
            None => anyhow!("No source folder set for this task"),
        });
    }
    Ok(merged)
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml walk_all a_source_that_cannot`
Expected: PASS. Then the full suite: `cargo test --manifest-path src-tauri/Cargo.toml` → 132 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/backup.rs
git commit -m "Walk every source into its own corner of the destination"
```

---

## Task 3: Wire the backup pipeline to `walk_all`

**Files:**
- Modify: `src-tauri/src/backup.rs` — `preflight_source` (:654), `execute_all` (:1178 onward)
- Modify: `src-tauri/src/backup.rs` test module — ~52 destination-path assertions

**Interfaces:**
- Consumes: `walk_all` (Task 2), `Task::sources()` (Task 1).
- Produces: test helper `fn backed_up(dest: &Path, src: &Path) -> PathBuf`, used by every test that asserts a destination path.

This is the task that changes the on-disk layout. It is bulky but cohesive: a reviewer either accepts "a backup now writes into a per-source subfolder" or does not.

- [ ] **Step 1: Add the test helper first, so the layout is written once**

In the test module, beside `scratch`:

```rust
    /// Where a source's file actually lands now: every source writes into a
    /// subfolder named after itself. Tests say this through the helper rather
    /// than spelling the layout 50 times, so changing it again is one edit.
    fn backed_up(dest: &Path, src: &Path) -> PathBuf {
        dest.join(src.file_name().expect("a source has a name"))
    }
```

- [ ] **Step 2: Write the failing end-to-end test**

```rust
    /// Two sources, one destination, two subtrees — and nothing of either
    /// source at the destination root, which is now the user's.
    #[tokio::test]
    async fn two_sources_land_in_two_subfolders_of_one_destination() {
        let root = scratch("two-sources-e2e");
        let a = root.join("Alpha");
        let b = root.join("Beta");
        let dest = root.join("dest");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(a.join("one.txt"), b"1").unwrap();
        std::fs::write(b.join("two.txt"), b"2").unwrap();

        let task = Task {
            id: "t".into(),
            name: "two sources".into(),
            source: None,
            sources: Some(vec![
                Source { path: a.to_string_lossy().into(), folder: "Alpha".into() },
                Source { path: b.to_string_lossy().into(), folder: "Beta".into() },
            ]),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };

        let app = tauri::test::mock_app();
        let payload = execute_all(
            app.handle(),
            "backup-two-sources",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(payload.success);
        assert_eq!(std::fs::read(dest.join("Alpha/one.txt")).unwrap(), b"1");
        assert_eq!(std::fs::read(dest.join("Beta/two.txt")).unwrap(), b"2");
        assert!(!dest.join("one.txt").exists(), "nothing lands at the destination root");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `desktop.ini` at a source root used to be the destination root's own
    /// icon marker, protected from prune on the grounds that the root's
    /// identity belongs to the user. After prefixing it is `Alpha/desktop.ini`
    /// and becomes that subfolder's icon descriptor instead — which is more
    /// correct, since the destination root now belongs to no single source,
    /// but it is a change and this is what pins it.
    #[tokio::test]
    async fn a_source_root_desktop_ini_becomes_the_subfolders_icon() {
        let root = scratch("source-icon");
        let a = root.join("Alpha");
        let dest = root.join("dest");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(a.join("desktop.ini"), b"[.ShellClassInfo]").unwrap();

        let task = Task {
            id: "icon".into(),
            name: "icon".into(),
            source: None,
            sources: Some(vec![Source {
                path: a.to_string_lossy().into(),
                folder: "Alpha".into(),
            }]),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let app = tauri::test::mock_app();
        execute_all(
            app.handle(),
            "backup-icon",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(
            dest.join("Alpha/desktop.ini").exists(),
            "it is copied like any other file, into its source's folder"
        );
        assert!(
            !is_root_icon_marker("Alpha/desktop.ini"),
            "and it no longer claims to be the destination root's marker"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Removing a source from a task must remove its subfolder on the next
    /// run — that is exactly what a per-source prune could not have done.
    #[tokio::test]
    async fn dropping_a_source_prunes_its_subfolder() {
        let root = scratch("drop-a-source");
        let a = root.join("Alpha");
        let dest = root.join("dest");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(dest.join("Beta")).unwrap();
        std::fs::write(a.join("one.txt"), b"1").unwrap();
        std::fs::write(dest.join("Beta/stale.txt"), b"old").unwrap();

        let keep = KeepSet::new(["Alpha/one.txt".to_string()].into_iter());
        let empty: HashSet<String> = HashSet::new();
        let mut stats = PhaseStats::default();
        prune_destination(
            &dest,
            &keep,
            &ProtectedSet::from_parts(&empty, &empty, &glob::PatternSet::new(&[])),
            &CancellationToken::new(),
            &mut stats,
        )
        .await
        .unwrap();

        assert!(!dest.join("Beta").exists(), "the dropped source's folder goes");
        let _ = std::fs::remove_dir_all(&root);
    }
```

`test_app()` is the existing helper the other end-to-end tests use; find it beside `run_one_destination` in the test module and reuse it verbatim.

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml two_sources_land dropping_a_source`
Expected: FAIL — files land at the destination root, not under `Alpha/`.

- [ ] **Step 4: Replace `preflight_source` with `preflight_sources`**

Existence is deliberately no longer checked here: a source on an unplugged drive must not fail the task, it must have its subfolder protected, which is `walk_all`'s job. What is fatal is a configuration that cannot be right on any disk.

```rust
/// The sources this task will read, with their *configuration* validated.
///
/// Existence is not checked here on purpose. A source whose drive is
/// unplugged must not fail the task — `walk_all` records it and prune walks
/// around its subfolder. What is fatal is a configuration no disk could make
/// right: no sources at all, a relative path, or a `folder` that is not a
/// usable folder name.
pub(crate) fn preflight_sources(task: &Task) -> Result<Vec<Source>> {
    let sources = task.sources();
    if sources.is_empty() {
        return Err(anyhow!("No source folder set for this task"));
    }
    for source in &sources {
        if !Path::new(&source.path).is_absolute() {
            return Err(anyhow!("Paths must be absolute"));
        }
        validate_folder_name(&source.folder)?;
    }
    Ok(sources)
}
```

`validate_folder_name` arrives in Task 4; for this task, add it as a stub that only rejects the empty string, and let Task 4 fill it in:

```rust
/// Filled in by the folder-name rules in the guards task.
fn validate_folder_name(folder: &str) -> Result<()> {
    if folder.trim().is_empty() {
        return Err(anyhow!("A source's destination folder cannot be empty"));
    }
    Ok(())
}
```

- [ ] **Step 5: Rewire `execute_all`**

At `backup.rs:1185-1191`, replace the single-source preamble:

```rust
    let sources = preflight_sources(task)?;
    let source_paths: Vec<PathBuf> = sources.iter().map(|s| PathBuf::from(&s.path)).collect();
    let destinations: Vec<PathBuf> = task.destinations().iter().map(PathBuf::from).collect();
    if destinations.is_empty() {
        return Err(anyhow!("No destination set for this task"));
    }
    reject_destination_overlaps(&source_paths, &destinations)?;
```

and at `:1206`, the walk:

```rust
    let mut walked = walk_all(&sources, &patterns, token).await?;
```

`reject_destination_overlaps` takes a slice as of Task 4; for this task, change its first parameter to `sources: &[PathBuf]` and wrap its existing body in `for source in sources { … }`. The source-versus-source rule arrives in Task 4.

- [ ] **Step 6: Update the existing assertions**

Every test that asserts a path under a destination gains one component. List them with `grep -n "dest\.join\|dest_root\.join\|destination\.join" src-tauri/src/backup.rs` and convert each:

```rust
// before
assert_eq!(std::fs::read(dest.join("docs/readme.md")).unwrap(), b"hello");
// after
assert_eq!(std::fs::read(backed_up(&dest, &source).join("docs/readme.md")).unwrap(), b"hello");
```

Leave `root.join(…)` and `source.join(…)` alone — those are the source side and do not move. Tests that build a `Task` literal also need `source: None, sources: Some(vec![Source { path: source.to_string_lossy().into(), folder: <source's basename>.into() }])`; where the source came from `scratch("name")`, that basename is `driveby-backup-test-name`, so `backed_up` is the only thing that should ever spell it.

Do the conversion in one pass, then run the suite. A test that fails after this is either a genuine regression or a path converted wrongly — both need reading, not another `join`.

- [ ] **Step 7: Run the full suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS, 135 tests (123 baseline + 4 + 5 + 3).
Then: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` → clean.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/backup.rs
git commit -m "Give every source its own folder at the destination"
```

---

## Task 4: The guards — what a multi-source task may not be

**Files:**
- Modify: `src-tauri/src/backup.rs` — `validate_folder_name`, `reject_destination_overlaps` (:748), `reject_foreign_overlaps` (:712)
- Test: `src-tauri/src/backup.rs`, test module (beside `nested_destinations_are_refused_before_anything_is_written` at :2404)

**Interfaces:**
- Consumes: `Source`, `preflight_sources` (Tasks 1 and 3).
- Produces: `fn validate_folder_name(folder: &str) -> Result<()>`; `reject_destination_overlaps(sources: &[PathBuf], destinations: &[PathBuf]) -> Result<()>` now also rejecting source-versus-source nesting.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Nesting one source inside another backs the inner one up twice, into
    /// two different subfolders. Refuse it the way every other overlap in
    /// this crate is refused, rather than quietly doing the work twice.
    #[test]
    fn nested_sources_are_refused() {
        let outer = PathBuf::from("/data");
        let inner = PathBuf::from("/data/photos");
        let dest = PathBuf::from("/backup");
        assert!(reject_destination_overlaps(&[outer, inner], &[dest]).is_err());
    }

    /// Two sources claiming the same subfolder would interleave at the
    /// destination and have prune fight itself. The comparison folds case
    /// where the filesystem does, so `Photos` and `photos` collide on NTFS
    /// and APFS — the same rule `KeepSet` uses.
    #[test]
    fn two_sources_cannot_claim_the_same_folder() {
        let task = Task {
            id: "t".into(),
            name: "t".into(),
            source: None,
            sources: Some(vec![
                Source { path: "/work/photos".into(), folder: "Photos".into() },
                Source { path: "/home/photos".into(), folder: "Photos".into() },
            ]),
            destination: None,
            destinations: None,
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        assert!(preflight_sources(&task).is_err());
    }

    /// A folder name is a name, not a path. `..` would climb out of the
    /// destination entirely and a separator would invent a hierarchy the
    /// user did not ask for — both write outside where the task was pointed.
    #[test]
    fn a_folder_name_that_is_really_a_path_is_refused() {
        for bad in ["", "   ", ".", "..", "a/b", "a\\b", "../escape"] {
            assert!(
                validate_folder_name(bad).is_err(),
                "{:?} should not be usable as a folder name",
                bad
            );
        }
        for good in ["Photos", "Photos-Work", "Mes documents", "2024.backup"] {
            assert!(validate_folder_name(good).is_ok(), "{:?} should be fine", good);
        }
    }

    /// The cross-task guard has to see every source, not just the first.
    /// A destination sitting on another task's second source would have this
    /// run prune away the files that task backs up from.
    #[test]
    fn a_destination_over_another_tasks_second_source_is_refused() {
        let other = Task {
            id: "other".into(),
            name: "other".into(),
            source: None,
            sources: Some(vec![
                Source { path: "/a".into(), folder: "A".into() },
                Source { path: "/b".into(), folder: "B".into() },
            ]),
            destination: None,
            destinations: Some(vec!["/elsewhere".into()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        assert!(reject_foreign_overlaps("mine", &[PathBuf::from("/b")], &[other]).is_err());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml nested_sources two_sources_cannot a_folder_name a_destination_over`
Expected: FAIL.

- [ ] **Step 3: Implement `validate_folder_name`**

Replace the Task 3 stub:

```rust
/// A source's destination subfolder is a folder *name*, not a path.
///
/// A separator would invent a hierarchy the user did not ask for, and `..`
/// would climb out of the destination altogether — both write outside the
/// folder the task was pointed at, which is the one thing a destination is
/// supposed to bound.
fn validate_folder_name(folder: &str) -> Result<()> {
    let name = folder.trim();
    if name.is_empty() {
        return Err(anyhow!("A source's destination folder cannot be empty"));
    }
    if name == "." || name == ".." {
        return Err(anyhow!("\"{}\" is not a folder name", name));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(anyhow!(
            "A source's destination folder is a name, not a path: \"{}\"",
            name
        ));
    }
    // The set Windows refuses outright. Checked on every platform so that a
    // task written on Linux does not fail only once it reaches a Windows
    // machine through a synced tasks.json.
    if name.contains(|c: char| "<>:\"|?*".contains(c) || (c as u32) < 0x20) {
        return Err(anyhow!(
            "A source's destination folder cannot contain <>:\"|?* : \"{}\"",
            name
        ));
    }
    Ok(())
}
```

- [ ] **Step 4: Add the duplicate-folder check to `preflight_sources`**

After the per-source loop:

```rust
    // Fully qualified: backup.rs imports HashSet, not HashMap, and `KeepSet`
    // right below already spells this type out the same way.
    let mut claimed: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for source in &sources {
        // Folded, because two names differing only in case are one folder on
        // the filesystems this app is mostly pointed at.
        if let Some(first) = claimed.insert(fold_rel(&source.folder), &source.path) {
            return Err(anyhow!(
                "Two sources cannot write to the same folder \"{}\": {} and {}",
                source.folder,
                first,
                source.path
            ));
        }
    }
```

- [ ] **Step 5: Add source-versus-source nesting to `reject_destination_overlaps`**

```rust
pub(crate) fn reject_destination_overlaps(
    sources: &[PathBuf],
    destinations: &[PathBuf],
) -> Result<()> {
    for (i, source) in sources.iter().enumerate() {
        // Nesting one source in another backs the inner one up twice, into
        // two subfolders, and every run copies it twice for ever.
        for other in &sources[i + 1..] {
            if path_contains(source, other) || path_contains(other, source) {
                return Err(anyhow!(
                    "Sources cannot be inside one another: {} and {}",
                    source.display(),
                    other.display()
                ));
            }
        }
        for (j, dest) in destinations.iter().enumerate() {
            reject_overlap(source, dest)?;
            for other in &destinations[j + 1..] {
                // path_contains is reflexive, so this also catches the same
                // folder listed twice under two spellings.
                if path_contains(dest, other) || path_contains(other, dest) {
                    return Err(anyhow!(
                        "Destinations cannot overlap: {} and {}",
                        dest.display(),
                        other.display()
                    ));
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Finish `reject_foreign_overlaps`**

Task 1 already made it iterate `other.sources()`. Confirm the loop covers every source and that the "contains the source folder of task X" message names the source it actually hit.

- [ ] **Step 7: Run and commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → 139 tests, then clippy.

```bash
git add src-tauri/src/backup.rs
git commit -m "Refuse the source lists that cannot mean what they say"
```

---

## Task 5: The preview pipeline

**Files:**
- Modify: `src-tauri/src/preview.rs:131-145` (`plan`)
- Test: `src-tauri/src/preview.rs`, test module (~7 destination-path assertions)

**Interfaces:**
- Consumes: `walk_all`, `preflight_sources` — both need `pub(crate)` visibility for `preview.rs` to import them.

`preview.rs` is **not** downstream of `execute_all`; it is a parallel pipeline that calls `preflight_source` and `walk` itself. If it is not converted, the preview reports on a layout the run does not produce — worse than no preview at all.

- [ ] **Step 1: Write the failing test**

```rust
    /// The preview has to describe the layout the run will actually produce,
    /// or the numbers in the confirmation dialog are about a different backup
    /// than the one about to happen.
    #[tokio::test]
    async fn a_preview_counts_every_source() {
        let root = scratch("preview-two-sources");
        let a = root.join("Alpha");
        let b = root.join("Beta");
        let dest = root.join("dest");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(a.join("one.txt"), b"1").unwrap();
        std::fs::write(b.join("two.txt"), b"2").unwrap();

        let task = task_with_sources(&[(&a, "Alpha"), (&b, "Beta")], &[&dest]);
        let payload = plan(&task, &Settings::default(), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(payload.source_files, 2, "both sources counted");
        assert_eq!(payload.destinations[0].new_files, 2);
        let _ = std::fs::remove_dir_all(&root);
    }
```

Add this helper beside the existing ones in `preview.rs`'s test module:

```rust
    fn task_with_sources(sources: &[(&Path, &str)], destinations: &[&Path]) -> Task {
        Task {
            id: "preview".into(),
            name: "preview".into(),
            source: None,
            sources: Some(
                sources
                    .iter()
                    .map(|(p, folder)| Source {
                        path: p.to_string_lossy().to_string(),
                        folder: (*folder).to_string(),
                    })
                    .collect(),
            ),
            destination: None,
            destinations: Some(
                destinations.iter().map(|d| d.to_string_lossy().to_string()).collect(),
            ),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        }
    }
```

`Source` needs adding to `preview.rs`'s `use crate::backup::{…}` list.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml a_preview_counts_every_source`
Expected: FAIL — only the first source is walked.

- [ ] **Step 3: Implement**

In `plan` (`preview.rs:132-143`), replace:

```rust
    let sources = preflight_sources(task)?;
    let source_paths: Vec<PathBuf> = sources.iter().map(|s| PathBuf::from(&s.path)).collect();
    let destinations: Vec<PathBuf> = task.destinations().iter().map(PathBuf::from).collect();
    if destinations.is_empty() {
        return Err(anyhow!("No destination set for this task"));
    }
    reject_destination_overlaps(&source_paths, &destinations)?;

    let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
    let walked = walk_all(&sources, &patterns, token).await?;
```

The rest of `plan` and all of `plan_one` are untouched: they read `walked` and `keep`, which now simply carry prefixed rels.

- [ ] **Step 4: Update the existing preview assertions**

`grep -n "dest\.join\|destination\.join" src-tauri/src/preview.rs` — seven of them. The preview tests build a single source; each asserted path gains that source's folder name.

- [ ] **Step 5: Run and commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` → 140 tests, then clippy.

```bash
git add src-tauri/src/preview.rs src-tauri/src/backup.rs
git commit -m "Preview the layout the run will actually write"
```

---

## Task 6: The frontend model — `src/lib/task.js`

**Files:**
- Modify: `src/lib/task.js`
- Test: `src/lib/__tests__/task.test.js`

**Interfaces:**
- Produces: `taskSources(task) -> [{path, folder}]`, `folderNameError(name) -> string|null`, `migrateTasks` extended, `findForeignOverlap(destinations, otherTasks, fold)` reading every source.

`taskSources` must agree with Rust's `Task::sources()` exactly — both read the same `tasks.json`, and the scheduler can tick before this migration has run.

- [ ] **Step 1: Write the failing tests**

```js
describe('taskSources', () => {
  it('folds a pre-multi-source task into one source named after its folder', () => {
    expect(taskSources({ source: 'C:/Photos' }))
      .toEqual([{ path: 'C:/Photos', folder: 'Photos' }]);
  });

  it('prefers the plural field and ignores the legacy one', () => {
    const task = {
      source: 'C:/Ignored',
      sources: [{ path: 'C:/Photos', folder: 'Pics' }],
    };
    expect(taskSources(task)).toEqual([{ path: 'C:/Photos', folder: 'Pics' }]);
  });

  it('drops blanks and exact repeats', () => {
    const task = {
      sources: [
        { path: 'C:/Photos', folder: 'Photos' },
        { path: '  ', folder: 'Blank' },
        { path: 'C:/Photos', folder: 'Photos' },
      ],
    };
    expect(taskSources(task)).toHaveLength(1);
  });

  it('gives no sources for a task that has neither field', () => {
    expect(taskSources({})).toEqual([]);
  });
});

describe('folderNameError', () => {
  it('refuses anything that is a path rather than a name', () => {
    for (const bad of ['', '   ', '.', '..', 'a/b', 'a\\b', 'x<y']) {
      expect(folderNameError(bad)).toBeTruthy();
    }
  });

  it('accepts ordinary folder names', () => {
    for (const good of ['Photos', 'Photos-Work', 'Mes documents', '2024.backup']) {
      expect(folderNameError(good)).toBeNull();
    }
  });
});

describe('migrateTasks with sources', () => {
  it('rewrites a legacy source into the plural shape and drops the old key', () => {
    const [out] = migrateTasks([{ id: '1', source: 'C:/Photos', destinations: ['D:/b'] }]);
    expect(out.sources).toEqual([{ path: 'C:/Photos', folder: 'Photos' }]);
    expect('source' in out).toBe(false);
  });

  it('is idempotent — a migrated list comes back as the same array', () => {
    const once = migrateTasks([{ id: '1', source: 'C:/Photos' }]);
    expect(migrateTasks(once)).toBe(once);
  });
});

describe('findForeignOverlap across every source', () => {
  it('catches a destination sitting on another task second source', () => {
    const other = {
      name: 'other',
      sources: [{ path: '/a', folder: 'A' }, { path: '/b', folder: 'B' }],
      destinations: ['/elsewhere'],
    };
    expect(findForeignOverlap(['/b'], [other])).toMatchObject({ kind: 'source', path: '/b' });
  });
});
```

Add `folderNameError` and `taskSources` to the import at the top of the test file.

- [ ] **Step 2: Run to verify they fail**

Run: `npm test`
Expected: FAIL — `taskSources` and `folderNameError` are not exported.

- [ ] **Step 3: Implement**

```js
/// The sources a task reads, in the order the user listed them, blanks
/// dropped and exact repeats collapsed.
///
/// Must match `Task::sources()` in src-tauri/src/backup.rs exactly: both
/// sides read the same tasks.json, and the scheduler can tick before the
/// frontend has rewritten it.
export function taskSources(task) {
  const listed = Array.isArray(task?.sources) && task.sources.length > 0
    ? task.sources
    : legacySource(task);
  const seen = new Set();
  return listed
    .filter((s) => s && typeof s.path === 'string' && typeof s.folder === 'string')
    .map((s) => ({ path: s.path.trim(), folder: s.folder.trim() }))
    .filter((s) => s.path && s.folder && !seen.has(s.path) && seen.add(s.path) !== false);
}

/// A pre-multi-source task's single source, named after its own folder. A
/// path with no final component — a bare drive root — cannot supply a name,
/// and yields nothing rather than an invented one.
function legacySource(task) {
  const path = typeof task?.source === 'string' ? task.source.trim() : '';
  if (!path) return [];
  const folder = path.replace(/[\\/]+$/, '').split(/[\\/]/).pop();
  return folder ? [{ path, folder }] : [];
}

/// Why this is not a usable destination folder name, or null when it is.
///
/// A name, not a path: a separator would invent a hierarchy nobody asked
/// for and `..` would climb out of the destination altogether. The refused
/// characters are the ones Windows rejects, checked everywhere so a task
/// written on Linux does not fail only once it reaches a Windows machine.
export function folderNameError(name) {
  const n = String(name ?? '').trim();
  if (!n) return 'empty';
  if (n === '.' || n === '..') return 'dots';
  if (/[\\/]/.test(n)) return 'separator';
  if (/[<>:"|?*\u0000-\u001f]/.test(n)) return 'character';
  return null;
}
```

Extend `needsMigration` to also fire on `'source' in task || !Array.isArray(task.sources)`, and `migrateTasks`'s mapper to strip `source` and add `sources: taskSources(task)` alongside what it already does for destinations.

In `findForeignOverlap`, replace `const theirSource = other?.source;` and its single check with a loop over `taskSources(other)`, testing `pathContains(mine, their.path, fold)` and returning `{ name, path: their.path, kind: 'source' }`.

- [ ] **Step 4: Run and commit**

Run: `npm test` → PASS.

```bash
git add src/lib/task.js src/lib/__tests__/task.test.js
git commit -m "Teach the frontend that a task has sources, plural"
```

---

## Task 7: The form, the cards, and the translations

**Files:**
- Modify: `src/components/NewTaskForm.jsx` (:17, :32, :40-43, :60, :95, :109, :161-173)
- Modify: `src/components/TaskCard.jsx:14`, `src/components/charts/TaskList.jsx:20`
- Modify: `src/context/AppContext.jsx:333`, `:336`
- Modify: `src/lib/i18n.js` — both the `en` block (~:84-110) and the `fr` block (~:314-340)

**Interfaces:**
- Consumes: `taskSources`, `folderNameError` (Task 6).

- [ ] **Step 1: Add the translation keys, both locales**

`en`:

```js
    'form.label.sources': 'Sources',
    'form.hint.sources': 'Each source is copied into its own folder at the destination',
    'form.action.add_source': 'Add a source',
    'form.action.remove_source': 'Remove',
    'form.aria.source': 'Source {n}',
    'form.aria.source_folder': 'Destination folder for {path}',
    'form.aria.remove_source': 'Remove source {path}',
    'form.error.source_overlap': 'Sources cannot be inside one another',
    'form.error.source_folder_duplicate': 'Two sources cannot use the same destination folder',
    'form.error.source_folder_invalid': 'A destination folder is a name, not a path',
```

`fr`:

```js
    'form.label.sources': 'Sources',
    'form.hint.sources': 'Chaque source est copiée dans son propre dossier à la destination',
    'form.action.add_source': 'Ajouter une source',
    'form.action.remove_source': 'Retirer',
    'form.aria.source': 'Source {n}',
    'form.aria.source_folder': 'Dossier de destination pour {path}',
    'form.aria.remove_source': 'Retirer la source {path}',
    'form.error.source_overlap': 'Les sources ne peuvent pas être imbriquées',
    'form.error.source_folder_duplicate': 'Deux sources ne peuvent pas utiliser le même dossier de destination',
    'form.error.source_folder_invalid': 'Un dossier de destination est un nom, pas un chemin',
```

Keep `form.error.source` ("Source folder required" / "Le dossier source est requis") — it still fires when the list is empty.

- [ ] **Step 2: Rework the form state and pickers**

`:17` — `source: ''` becomes `sources: []`. `:32` — `source: initialTask.source || ''` becomes `sources: taskSources(initialTask)`.

Replace `pickSource` with the list version, modelled on `pickDestination` (which is the reason overlaps are checked at pick time: the message can then name the folder that is the problem):

```jsx
  const pickSource = async (index) => {
    const picked = await bridge.selectDirectory(t('form.dialog.select_source'));
    if (!picked) return;
    const others = task.sources.filter((_, i) => i !== index);
    if (others.some((s) => pathContains(s.path, picked) || pathContains(picked, s.path))) {
      showToast?.(t('form.error.source_overlap'), 'error');
      return;
    }
    if (task.destinations.some((d) => pathContains(d, picked) || pathContains(picked, d))) {
      showToast?.(t('form.error.dest_in_source'), 'error');
      return;
    }
    const folder = picked.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || '';
    setTask((prev) => {
      const sources = [...prev.sources];
      const next = { path: picked, folder };
      if (index === null || index >= sources.length) sources.push(next);
      else sources[index] = next;
      return { ...prev, sources };
    });
  };

  const setSourceFolder = (index, folder) =>
    setTask((prev) => ({
      ...prev,
      sources: prev.sources.map((s, i) => (i === index ? { ...s, folder } : s)),
    }));

  const removeSource = (index) =>
    setTask((prev) => ({ ...prev, sources: prev.sources.filter((_, i) => i !== index) }));
```

`pickDestination` at `:60` checks against one `task.source`; change to `task.sources.some((s) => pathContains(s.path, picked) || pathContains(picked, s.path))`.

- [ ] **Step 3: Rework `submit`'s refusals**

`:95` — `if (!task.source)` becomes `if (task.sources.length === 0)`, same message key.

`:109` — the destination-in-source check iterates sources.

Add, before the foreign-overlap check:

```jsx
    if (findOverlap(task.sources.map((s) => s.path))) {
      return showToast?.(t('form.error.source_overlap'), 'error');
    }
    if (task.sources.some((s) => folderNameError(s.folder))) {
      return showToast?.(t('form.error.source_folder_invalid'), 'error');
    }
    const folders = task.sources.map((s) => s.folder.trim().toLowerCase());
    if (new Set(folders).size !== folders.length) {
      return showToast?.(t('form.error.source_folder_duplicate'), 'error');
    }
```

- [ ] **Step 4: Replace the Source field with the list**

Model it on the `dest-list` block directly below it, reusing `field-row`, with the editable folder input added:

```jsx
      <FormField label={t('form.label.sources')} hint={t('form.hint.sources')}>
        <div className="dest-list">
          {task.sources.map((source, i) => (
            <div className="field-row" key={`${i}-${source.path}`}>
              <input
                className="field field--readonly"
                readOnly
                value={source.path}
                title={source.path}
                aria-label={t('form.aria.source', { n: i + 1 })}
                autoComplete="off"
                name={`driveby-task-source-${i}`}
              />
              <Button size="small" onClick={() => pickSource(i)}>{t('common.choose')}</Button>
              <input
                className="field field--narrow"
                value={source.folder}
                onChange={(e) => setSourceFolder(i, e.target.value)}
                aria-label={t('form.aria.source_folder', { path: source.path })}
                aria-invalid={!!folderNameError(source.folder)}
                autoComplete="off"
                spellCheck={false}
                name={`driveby-task-source-folder-${i}`}
              />
              <Button
                size="small"
                variant="borderless"
                destructive
                onClick={() => removeSource(i)}
                ariaLabel={t('form.aria.remove_source', { path: source.path })}
              >
                {t('form.action.remove_source')}
              </Button>
            </div>
          ))}
          <Button size="small" variant="borderless" onClick={() => pickSource(null)}>
            {t('form.action.add_source')}
          </Button>
        </div>
      </FormField>
```

Add `.field--narrow { flex: 0 0 12rem; }` beside the existing `.field--readonly` rule in the stylesheet that defines it.

- [ ] **Step 5: Update the cards and the draft check**

`TaskCard.jsx:14` and `TaskList.jsx:20` both build `` `${task.source} → …` ``. Replace with:

```jsx
  const paths = `${taskSources(task).map((s) => s.path).join(', ')} → ${taskDestinations(task).join(', ')}`;
```

`AppContext.jsx:333` — `!taskDraft.source` becomes `taskSources(taskDraft).length === 0`. `:336` — the created task carries `sources` rather than `source`.

- [ ] **Step 6: Run everything**

Run: `npm test` → 84+ passing. Then `npm run build` to catch an unresolved import, and `cargo test --manifest-path src-tauri/Cargo.toml`.

- [ ] **Step 7: Add the CHANGELOG note**

The reshape is a change of on-disk shape and belongs in the release body. The version number is the user's to choose, so write the entry under a heading they name at release time; the prose is:

> Tasks can now back up several source folders to one destination. Each source is copied into its own folder at the destination, named after the source — **including tasks with a single source**. The first run after updating therefore moves an existing backup one level down, which it does by deleting and re-copying: expect one long run, and check the preview's "deleted" count before confirming it.

- [ ] **Step 8: Commit**

```bash
git add src/ CHANGELOG.md
git commit -m "Let the form take a list of sources"
```

---

## Verification

**Per task:** the named test fails, then passes, then the whole suite passes.

**Before pushing:**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm test
npm run build
```

Do **not** run `cargo fmt`.

**Manual, in the running app** (`npm run tauri dev`) — none of these are reachable from the test suite:

1. **Two sources, one destination.** Make a task with `Photos` and `Documents` pointing at one folder. Run it. The destination holds `Photos/` and `Documents/`, nothing at its root.
2. **The reshape, seen through the preview.** Open a task that ran under 1.7.4 and press Run with `confirmBeforeBackup` on. The dialog reports every file as deleted *and* every file as new — that is the reshape, in real numbers, before it happens.
3. **A source on an unplugged drive.** Two sources, one on a removable disk. Unplug it and run. The destination keeps that source's folder untouched, the other source backs up normally, and the run reports itself incomplete rather than successful.
4. **A dropped source.** Remove a source from a task and run. Its folder disappears from the destination.
5. **The refusals.** Try nesting one source in another, and two sources with the same folder name — both refused at the moment of picking, naming the offending folder.

**Then:** push `master` and wait for CI on Windows, macOS and Linux. The release decision is separate.
