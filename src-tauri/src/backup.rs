use crate::fsutil::{
    apply_attrs, clear_readonly, long_path, read_attrs, reject_overlap, ATTR_KEEP,
};
use crate::glob;
use crate::persist;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use filetime::FileTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use xxhash_rust::xxh3::Xxh3;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default, rename = "lastBackup")]
    pub last_backup: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    #[serde(default, rename = "excludePatterns")]
    pub exclude_patterns: String,
    #[serde(default, rename = "showNotifications")]
    pub show_notifications: bool,
    #[serde(default, rename = "verify")]
    pub verify: Option<bool>,
    #[serde(default, rename = "continueOnError")]
    pub continue_on_error: Option<bool>,
    #[serde(default, rename = "preserveMtime")]
    pub preserve_mtime: Option<bool>,
}

impl Settings {
    fn verify(&self) -> bool {
        self.verify.unwrap_or(false)
    }
    fn continue_on_error(&self) -> bool {
        self.continue_on_error.unwrap_or(true)
    }
    fn preserve_mtime(&self) -> bool {
        self.preserve_mtime.unwrap_or(true)
    }
}

#[derive(Default)]
pub struct BackupState {
    active: Arc<DashMap<String, CancellationToken>>,
}

impl BackupState {
    pub fn cancel(&self, task_id: &str) {
        if let Some((_, token)) = self.active.remove(task_id) {
            token.cancel();
        }
    }
    pub fn is_active(&self, task_id: &str) -> bool {
        self.active.contains_key(task_id)
    }
    /// Atomic register-if-absent. Returns None when a backup is already
    /// running for this task — the caller must not start a second one.
    fn try_register(&self, task_id: &str) -> Option<CancellationToken> {
        let token = CancellationToken::new();
        // DashMap::entry gives us a single locked slot; insert only if vacant.
        match self.active.entry(task_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(_) => None,
            dashmap::mapref::entry::Entry::Vacant(v) => {
                v.insert(token.clone());
                Some(token)
            }
        }
    }
    fn unregister(&self, task_id: &str) {
        self.active.remove(task_id);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StartedPayload {
    backup_id: String,
    task_id: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    backup_id: String,
    task_id: String,
    progress: u32,
    copied_bytes: u64,
    total_bytes: u64,
    copied_files: u64,
    total_files: u64,
    speed_bps: u64,
    eta_seconds: Option<u64>,
    phase: &'static str,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompletePayload {
    pub backup_id: String,
    pub task_id: String,
    pub success: bool,
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleaned: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unchanged: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    /// Source entries the walk could not enumerate or stat. Their
    /// destination counterparts are deliberately left untouched by the
    /// prune pass, so a non-zero value means "this backup is knowingly
    /// incomplete but nothing was deleted for it".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<u64>,
}

struct FileEntry {
    path: PathBuf,
    rel: String,
    size: u64,
    mtime: SystemTime,
}

/// Cooperative-cancellation sentinel string used by the inner `execute()`
/// pipeline to short-circuit on `CancellationToken::is_cancelled()`. The
/// outer `run_backup()` no longer string-matches this — it consults the
/// token directly — but having a single named constant keeps the in-function
/// returns consistent and greppable.
const CANCELLED_MSG: &str = "backup cancelled";

// ─────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────

pub async fn run_backup(
    app: &AppHandle,
    state: &BackupState,
    task: Task,
    settings: Settings,
) -> Result<CompletePayload> {
    let backup_id = uuid::Uuid::new_v4().to_string();

    // Atomic guard: refuse a second concurrent run for the same task. Without
    // this, double-click or scheduler-while-manual would race two writers
    // against the same destination tree (#1).
    let token = match state.try_register(&task.id) {
        Some(t) => t,
        None => {
            return Err(anyhow!("A backup is already running for this task"));
        }
    };

    let result = execute(app, &backup_id, &task, &settings, &token).await;

    // Snapshot cancellation state from the token *before* unregister, so we
    // don't depend on stringly-typed error matching (`err.to_string().contains
    // ("ABORTED")`). The token is the source of truth: a successful execute()
    // always returns Ok, a cancelled one always has the token tripped, and
    // any other failure leaves the token alone.
    let cancelled = token.is_cancelled();
    state.unregister(&task.id);

    let payload = match result {
        Ok(p) => p,
        Err(err) => CompletePayload {
            backup_id: backup_id.clone(),
            task_id: task.id.clone(),
            success: false,
            cancelled,
            error: if cancelled {
                None
            } else {
                Some(err.to_string())
            },
            path: None,
            total_bytes: None,
            total_files: None,
            duration_ms: None,
            skipped: None,
            cleaned: None,
            unchanged: None,
            failed: None,
            verified: None,
            unreadable: None,
        },
    };

    // Persist lastBackup in Rust, emit task-updated (centralized ownership).
    // Only on success — partial failures shouldn't reset the schedule clock,
    // otherwise a permanently-broken task hides behind a fresh timestamp and
    // the user never notices it's stuck.
    if !payload.cancelled && payload.success {
        let upd = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            update_last_backup(app, &task.id),
        )
        .await;
        match upd {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!("could not persist lastBackup: {}", e),
            Err(_) => warn!("lastBackup persist timed out"),
        }
    }

    info!(task = %task.name, success = payload.success, "emitting backup-complete");
    let _ = app.emit("backup-complete", payload.clone());
    Ok(payload)
}

async fn update_last_backup(app: &AppHandle, task_id: &str) -> Result<()> {
    let dir = app.path().app_data_dir().ok();
    let Some(dir) = dir else {
        return Ok(());
    };
    let path = dir.join("tasks.json");
    // Serialise with the JS-side writer via the shared persist mutex (#7).
    persist::with_tasks_lock(|| async {
        let mut value: serde_json::Value =
            persist::read_json_or(&path, serde_json::Value::Array(vec![])).await;
        let now = Utc::now().to_rfc3339();
        let mut updated_task: Option<serde_json::Value> = None;
        if let Some(arr) = value.as_array_mut() {
            for t in arr.iter_mut() {
                if t.get("id").and_then(|v| v.as_str()) == Some(task_id) {
                    t["lastBackup"] = serde_json::Value::String(now.clone());
                    updated_task = Some(t.clone());
                }
            }
        }
        persist::write_json_atomic(&path, &value).await?;
        if let Some(t) = updated_task {
            let _ = app.emit("task-updated", t);
        }
        Ok(())
    })
    .await
}

// ─────────────────────────────────────────────────────────────────────
// Main pipeline
// ─────────────────────────────────────────────────────────────────────
/// The immutable facts about a run, plus the progress channel back to the
/// UI. Every phase gets one of these instead of the eight loose parameters
/// each of them used to need.
struct RunCtx<'a> {
    app: &'a AppHandle,
    backup_id: &'a str,
    task_id: &'a str,
    target: &'a Path,
    settings: &'a Settings,
    token: &'a CancellationToken,
    started: Instant,
    total_bytes: u64,
    total_files: u64,
}

impl RunCtx<'_> {
    /// Err(CANCELLED_MSG) if the user asked to stop. Phases call this at
    /// every point where stopping is safe.
    fn check_cancelled(&self) -> Result<()> {
        if self.token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        Ok(())
    }

    /// Unconditional progress emit — used at phase boundaries, where there
    /// is one event rather than a stream and the throttle would only risk
    /// swallowing it.
    fn emit_phase(
        &self,
        phase: &'static str,
        copied_bytes: u64,
        copied_files: u64,
        eta_seconds: Option<u64>,
    ) {
        let _ = self.app.emit(
            "backup-progress",
            ProgressPayload {
                backup_id: self.backup_id.to_string(),
                task_id: self.task_id.to_string(),
                progress: 100,
                copied_bytes,
                total_bytes: self.total_bytes,
                copied_files,
                total_files: self.total_files,
                speed_bps: 0,
                eta_seconds,
                phase,
            },
        );
    }

    /// Throttled progress emit for the copy loop: at most one event per
    /// 100ms, with speed and ETA derived from elapsed wall time.
    fn maybe_emit(
        &self,
        last_emit: &mut Instant,
        copied_bytes: u64,
        copied_files: u64,
        phase: &'static str,
    ) {
        if last_emit.elapsed().as_millis() < 100 {
            return;
        }
        *last_emit = Instant::now();
        let elapsed = self.started.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            (copied_bytes as f64 / elapsed) as u64
        } else {
            0
        };
        // `checked_div` returns None when speed is 0, sparing us the manual
        // guard *and* the potential div-by-zero panic if speed underflowed.
        let eta = self
            .total_bytes
            .saturating_sub(copied_bytes)
            .checked_div(speed);
        let progress = if self.total_bytes > 0 {
            ((copied_bytes as f64 / self.total_bytes as f64) * 100.0).min(100.0) as u32
        } else {
            0
        };
        let _ = self.app.emit(
            "backup-progress",
            ProgressPayload {
                backup_id: self.backup_id.to_string(),
                task_id: self.task_id.to_string(),
                progress,
                copied_bytes,
                total_bytes: self.total_bytes,
                copied_files,
                total_files: self.total_files,
                speed_bps: speed,
                eta_seconds: eta,
                phase,
            },
        );
    }
}

/// Running totals threaded through the phases and folded into the final
/// payload. One accumulator rather than a dozen `let mut` bindings living
/// for the whole length of `execute()`.
#[derive(Default)]
struct PhaseStats {
    copied_bytes: u64,
    copied_files: u64,
    unchanged: u64,
    failed: u64,
    deleted: u64,
    recased: u64,
    icon_resyncs: u64,
    attr_drift: u64,
    errors: Vec<String>,
}

/// Validate the pair of paths before touching anything. Returns them as
/// owned PathBufs so the caller doesn't re-parse the task strings.
async fn preflight(task: &Task) -> Result<(PathBuf, PathBuf)> {
    let source = PathBuf::from(&task.source);
    let destination = PathBuf::from(&task.destination);

    if !source.is_absolute() || !destination.is_absolute() {
        return Err(anyhow!("Paths must be absolute"));
    }
    let src_meta = fs::metadata(long_path(&source))
        .await
        .map_err(|_| anyhow!("Source folder not found"))?;
    if !src_meta.is_dir() {
        return Err(anyhow!("Source is not a directory"));
    }
    let dest_meta = fs::metadata(long_path(&destination))
        .await
        .map_err(|_| anyhow!("Destination not found"))?;
    if !dest_meta.is_dir() {
        return Err(anyhow!("Destination is not a directory"));
    }
    // Reject self-syncs and any nested overlap (#3). Shared with restore(),
    // which needs the same guard for an even sharper reason.
    reject_overlap(&source, &destination)?;
    Ok((source, destination))
}

/// Mirror the source files into the destination. Files already present with
/// matching size + mtime (within the 2s tolerance) are skipped.
async fn copy_phase(ctx: &RunCtx<'_>, files: &[FileEntry], stats: &mut PhaseStats) -> Result<()> {
    let mut copied_bytes: u64 = 0;
    let mut copied_files: u64 = 0;
    let mut last_emit = Instant::now();

    for file in files {
        ctx.check_cancelled()?;
        let dest_path = ctx.target.join(&file.rel);
        if let Some(parent) = dest_path.parent() {
            // Don't kill the whole job because one parent can't be made (#8).
            if let Err(e) = fs::create_dir_all(long_path(parent)).await {
                stats.failed += 1;
                warn!(target = %file.rel, "create parent failed: {}", e);
                stats
                    .errors
                    .push(format!("{}: create parent: {}", file.rel, e));
                if !ctx.settings.continue_on_error() {
                    return Err(anyhow!("create parent for {}: {}", file.rel, e));
                }
                continue;
            }
        }

        // Skip if destination already has an identical file (size + mtime
        // match within tolerance — see same_mtime). EXCEPT folder-icon
        // descriptors (`desktop.ini`): always re-copy them so a stale or
        // tampered destination copy can never silently leave a folder
        // rendering with the wrong icon. They're tiny (typically <1 KB).
        let skip = !is_icon_descriptor(&file.rel)
            && match fs::metadata(long_path(&dest_path)).await {
                Ok(meta) => {
                    meta.is_file()
                        && meta.len() == file.size
                        && meta
                            .modified()
                            .ok()
                            .is_some_and(|m| same_mtime(m, file.mtime))
                }
                Err(_) => false,
            };

        if skip {
            stats.unchanged += 1;
            copied_bytes += file.size;
            copied_files += 1;
            ctx.maybe_emit(&mut last_emit, copied_bytes, copied_files, "syncing");
            continue;
        }

        // Track bytes within this file so retries don't double-count toward
        // the global total — progress would overshoot and "stick" at 100%
        // while the loop kept copying. The callback receives this file's
        // cumulative bytes so far (resets to 0 on a retry).
        let base_bytes = copied_bytes;
        let result = copy_with_retries(
            &file.path,
            &dest_path,
            ctx.token,
            ctx.settings,
            |file_so_far| {
                copied_bytes = base_bytes + file_so_far;
                ctx.maybe_emit(&mut last_emit, copied_bytes, copied_files, "copying");
            },
        )
        .await;

        match result {
            Ok(()) => {
                copied_files += 1;
                copied_bytes = base_bytes + file.size;
            }
            Err(e) => {
                ctx.check_cancelled()?;
                stats.failed += 1;
                copied_bytes = base_bytes; // discard partial progress for failed file
                warn!(target = %file.rel, "copy failed: {}", e);
                stats.errors.push(format!("{}: {}", file.rel, e));
                if !ctx.settings.continue_on_error() {
                    return Err(e);
                }
            }
        }
    }

    stats.copied_bytes = copied_bytes;
    stats.copied_files = copied_files;
    Ok(())
}

/// Mirror-delete pass: remove anything in the destination that is no longer
/// in the source. Excluded paths are *preserved* — exclude means "don't
/// copy", not "delete from dest" (#2) — as are paths the walk could not read.
async fn prune_phase(
    ctx: &RunCtx<'_>,
    files: &[FileEntry],
    walked: &WalkResult,
    patterns: &glob::PatternSet,
    stats: &mut PhaseStats,
) -> Result<()> {
    ctx.emit_phase("pruning", stats.copied_bytes, stats.copied_files, None);
    let keep = KeepSet::new(files.iter().map(|f| f.rel.clone()));
    if let Err(e) = prune_destination(
        ctx.target,
        &keep,
        &walked.excluded,
        &walked.unreadable,
        patterns,
        ctx.token,
        stats,
    )
    .await
    {
        ctx.check_cancelled()?;
        warn!("prune destination failed: {}", e);
    }
    if stats.recased > 0 {
        info!(
            "restored source casing on {} destination file(s)",
            stats.recased
        );
    }
    Ok(())
}

/// Folder-icon hash verification.
///
/// Custom Windows folder icons live in two places: a `desktop.ini` *file*
/// with a `[.ShellClassInfo]` block, and the *parent folder's* Readonly or
/// System bit. Both halves must match the source byte-for-byte for Explorer
/// to render the correct icon. The copy loop already always re-copies every
/// `desktop.ini`, but disk errors, power loss between flush and sync_all,
/// and filesystem quirks can still produce a "copy succeeded" file that
/// doesn't actually equal the source. We close that gap by hashing both
/// sides via xxh3 and force-re-copying any drift. This runs *before* the
/// parent-folder attribute apply, so the very last action on each icon
/// folder is setting the `+R`/`+S` bit on a folder whose `desktop.ini` is
/// known to match the source bit-for-bit.
async fn verify_icons_phase(
    ctx: &RunCtx<'_>,
    files: &[FileEntry],
    stats: &mut PhaseStats,
) -> Result<()> {
    let icon_files: Vec<&FileEntry> = files
        .iter()
        .filter(|f| is_icon_descriptor(&f.rel))
        .collect();
    if icon_files.is_empty() {
        return Ok(());
    }

    ctx.emit_phase(
        "verifying-icons",
        stats.copied_bytes,
        stats.copied_files,
        None,
    );
    for f in &icon_files {
        ctx.check_cancelled()?;
        let dest_path = ctx.target.join(&f.rel);
        let src_hash = match hash_file(&f.path).await {
            Ok(h) => h,
            Err(e) => {
                warn!("could not hash source icon descriptor {}: {}", f.rel, e);
                continue;
            }
        };
        let dst_hash = if fs::metadata(long_path(&dest_path)).await.is_ok() {
            hash_file(&dest_path).await.ok()
        } else {
            None
        };
        if Some(src_hash) != dst_hash {
            warn!(
                "folder-icon descriptor {} differs from source — forcing re-copy",
                f.rel
            );
            if let Some(parent) = dest_path.parent() {
                let _ = fs::create_dir_all(long_path(parent)).await;
            }
            match copy_with_retries(&f.path, &dest_path, ctx.token, ctx.settings, |_| {}).await {
                Ok(()) => stats.icon_resyncs += 1,
                Err(e) => warn!("icon descriptor resync of {} failed: {}", f.rel, e),
            }
        }
    }
    if stats.icon_resyncs > 0 {
        info!(
            "re-synced {} folder-icon descriptor(s) after hash mismatch",
            stats.icon_resyncs
        );
    } else {
        info!(
            "verified {} folder-icon descriptor(s) — all match source",
            icon_files.len()
        );
    }
    Ok(())
}

/// Mirror directory attributes — the second half of "custom folder icon",
/// the one that lives on the parent folder.
///
/// Sorted deepest-first so a child's bits are applied before any ancestor
/// inherits a Readonly flag that would block mutation, and every source dir
/// gets a destination counterpart (empty source subfolders aren't created by
/// the copy loop). After applying, the destination attrs are read back and a
/// mismatch is warned about — that's what surfaces filesystem-level
/// limitations (e.g. exFAT silently dropping `+R` on directories) rather
/// than letting the destination quietly render a default icon.
async fn mirror_dir_attrs_phase(
    ctx: &RunCtx<'_>,
    dirs: &mut [(PathBuf, String)],
    stats: &mut PhaseStats,
) {
    // Sort in place — `dirs` is not needed in source order after this point.
    dirs.sort_by_key(|(_, r)| std::cmp::Reverse(r.len()));
    for (src_dir, rel) in dirs.iter() {
        let dest_dir = ctx.target.join(rel);
        if let Err(e) = fs::create_dir_all(long_path(&dest_dir)).await {
            warn!("could not ensure destination dir {}: {}", rel, e);
            continue;
        }
        let Some(src_attrs) = read_attrs(src_dir) else {
            continue;
        };
        let want = src_attrs & ATTR_KEEP;
        apply_attrs(&dest_dir, src_attrs);
        if want == 0 {
            continue;
        }
        if let Some(got_full) = read_attrs(&dest_dir) {
            let got = got_full & ATTR_KEEP;
            if got != want {
                stats.attr_drift += 1;
                warn!(
                    "destination folder attrs at {} did not stick: want={:#x} got={:#x} (filesystem may not support these bits)",
                    rel, want, got
                );
            }
        }
    }
    if stats.attr_drift > 0 {
        warn!("{} destination folder(s) could not mirror source attributes — custom icons there may not render", stats.attr_drift);
    }
}

/// Optional whole-tree hash verification. Only runs on a clean copy pass:
/// re-reading files we already know failed would just report the failure a
/// second time, more slowly.
async fn verify_phase(ctx: &RunCtx<'_>, files: &[FileEntry], stats: &PhaseStats) -> Result<bool> {
    if !ctx.settings.verify() || stats.failed != 0 {
        return Ok(false);
    }
    ctx.emit_phase("verifying", stats.copied_bytes, stats.copied_files, None);
    verify_files(files, ctx.target, ctx.token).await?;
    Ok(true)
}

async fn execute(
    app: &AppHandle,
    backup_id: &str,
    task: &Task,
    settings: &Settings,
    token: &CancellationToken,
) -> Result<CompletePayload> {
    let (source, destination) = preflight(task).await?;

    let _ = app.emit(
        "backup-started",
        StartedPayload {
            backup_id: backup_id.to_string(),
            task_id: task.id.clone(),
        },
    );

    info!(task = %task.name, "walking source");
    let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
    let mut walked = walk(&source, &patterns).await?;
    if !walked.unreadable.is_empty() {
        warn!(
            "{} source path(s) could not be read — their destination copies are left untouched",
            walked.unreadable.len()
        );
    }

    let ctx = RunCtx {
        app,
        backup_id,
        task_id: &task.id,
        target: &destination,
        settings,
        token,
        started: Instant::now(),
        total_bytes: walked.total_bytes,
        total_files: walked.files.len() as u64,
    };
    let mut stats = PhaseStats::default();

    copy_phase(&ctx, &walked.files, &mut stats).await?;
    prune_phase(&ctx, &walked.files, &walked, &patterns, &mut stats).await?;
    verify_icons_phase(&ctx, &walked.files, &mut stats).await?;
    mirror_dir_attrs_phase(&ctx, &mut walked.dirs, &mut stats).await;

    // Force a final 100% emit so the UI reflects completion even when the
    // throttle would have skipped the last chunk.
    ctx.emit_phase("finishing", ctx.total_bytes, ctx.total_files, Some(0));

    let verified = verify_phase(&ctx, &walked.files, &stats).await?;

    for e in &stats.errors {
        warn!("file error: {}", e);
    }

    Ok(CompletePayload {
        backup_id: backup_id.to_string(),
        task_id: task.id.clone(),
        success: stats.failed == 0,
        cancelled: false,
        error: if stats.failed > 0 {
            Some(format!("{} file(s) failed", stats.failed))
        } else {
            None
        },
        path: Some(destination.to_string_lossy().to_string()),
        total_bytes: Some(ctx.total_bytes),
        total_files: Some(ctx.total_files),
        duration_ms: Some(ctx.started.elapsed().as_millis() as u64),
        skipped: Some(walked.skipped as u64),
        cleaned: Some(stats.deleted),
        unchanged: Some(stats.unchanged),
        failed: Some(stats.failed),
        verified: Some(verified),
        unreadable: Some(walked.unreadable.len() as u64),
    })
}

/// Walk `root` and remove any file whose relative path is not present in
/// `keep` AND not protected by `excluded` / `unreadable` / `patterns`.
///
/// Excluded paths are preserved so that adding `node_modules` to the exclude
/// list never wipes pre-existing destination data (#2). Unreadable paths are
/// preserved for a sharper reason: a source directory we failed to enumerate
/// contributes nothing to `keep`, so without this guard a transient
/// permission error or a locked folder would make prune delete that entire
/// subtree from the destination — and the run would still report success.
///
/// Note that the guard skips the entry *without pushing it on the stack*, so
/// protecting a directory protects its whole subtree. Skipped on cancellation.
#[allow(clippy::too_many_arguments)]
async fn prune_destination(
    root: &Path,
    keep: &KeepSet,
    excluded: &HashSet<String>,
    unreadable: &HashSet<String>,
    patterns: &glob::PatternSet,
    token: &CancellationToken,
    stats: &mut PhaseStats,
) -> Result<()> {
    // An empty relative path is the root's own. Its presence means the source
    // root itself could not be enumerated, so *nothing* in the destination
    // can be shown to be orphaned — the entry-by-entry guard below would
    // never match it, which is what once made prune wipe whole destinations.
    if unreadable.contains("") {
        warn!("source root was unreadable — skipping prune entirely");
        return Ok(());
    }

    let mut dirs_to_check: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![long_path(root)];
    let root_canonical = long_path(root);

    while let Some(dir) = stack.pop() {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let mut entries = match fs::read_dir(&dir).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        loop {
            if token.is_cancelled() {
                return Err(anyhow!(CANCELLED_MSG));
            }
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => break,
            };
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            let rel_str = rel_of(&root_canonical, &path);

            // Never touch destination paths the user told us to leave alone,
            // nor those the source walk could not read — and don't recurse
            // into them either, since pruning their contents would still
            // effectively delete protected data.
            if excluded.contains(&rel_str)
                || unreadable.contains(&rel_str)
                || patterns.matches(&rel_str)
            {
                continue;
            }

            if file_type.is_dir() {
                stack.push(path.clone());
                dirs_to_check.push(path);
            } else if file_type.is_file() {
                match keep.status(&rel_str) {
                    KeepStatus::Exact => {}
                    KeepStatus::CaseDrift(source_rel) => {
                        // NTFS is case-insensitive but case-*preserving*, so
                        // when the user re-cases a source file the copy loop
                        // writes through to the existing directory entry and
                        // leaves the old spelling on disk. prune then failed
                        // to find that spelling in `keep` and deleted the
                        // file it had just copied — the backup silently lost
                        // it until the following run. Re-spell instead.
                        recase_entry(&path, source_rel, stats).await;
                    }
                    KeepStatus::Absent => {
                        // A destination file that inherited READONLY from a
                        // source file that has since been deleted would
                        // otherwise be un-prunable forever.
                        clear_readonly(&path);
                        if fs::remove_file(&path).await.is_ok() {
                            stats.deleted += 1;
                        }
                    }
                }
            }
        }
    }

    // Bottom-up: remove now-empty directories. Sort deepest-first by length.
    dirs_to_check.sort_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
    for d in dirs_to_check {
        // succeeds only if empty
        if fs::remove_dir(&d).await.is_ok() {
            continue;
        }
        // Custom-icon folders carry `+R`, which blocks RemoveDirectoryW —
        // but stripping it up front cleared the bit from every directory we
        // merely walked past. Only `mirror_dir_attrs_phase` restores it, two
        // phases later, and the icon-verification phase in between can bail
        // out on cancellation: pressing Stop then left the whole destination
        // rendering with default folder icons. Strip it only when a removal
        // actually needs it, and put it back if the directory stays.
        let attrs = read_attrs(&d);
        clear_readonly(&d);
        if fs::remove_dir(&d).await.is_err() {
            if let Some(a) = attrs {
                apply_attrs(&d, a);
            }
        }
    }
    Ok(())
}

/// Rename a destination entry to the spelling the source uses.
///
/// Only the file name is re-spelled: if the case drift is in a *directory*
/// component the basename already matches, `with_file_name` yields the path
/// we started from, and we leave it alone. Directory-level case drift is not
/// corrected — but the file is no longer deleted for it either, which was
/// the part that lost data.
async fn recase_entry(path: &Path, source_rel: &str, stats: &mut PhaseStats) {
    let Some(name) = Path::new(source_rel).file_name() else {
        return;
    };
    let target = path.with_file_name(name);
    if target == path {
        return; // drift is in a parent component, not the file name
    }
    match fs::rename(path, &target).await {
        Ok(()) => stats.recased += 1,
        Err(e) => warn!(
            "could not restore source casing for {}: {}",
            path.display(),
            e
        ),
    }
}

/// Compare mtimes with a 2-second tolerance. FAT/exFAT (very common on
/// external backup drives) rounds mtime to even seconds, so the round-trip
/// `set_file_mtime(dest, src)` can land 1s off the source — without this
/// tolerance every sync re-copies every file (#4).
fn same_mtime(a: SystemTime, b: SystemTime) -> bool {
    let da = a.duration_since(std::time::UNIX_EPOCH).ok();
    let db = b.duration_since(std::time::UNIX_EPOCH).ok();
    match (da, db) {
        (Some(x), Some(y)) => {
            let xs = x.as_secs() as i64;
            let ys = y.as_secs() as i64;
            (xs - ys).abs() <= 2
        }
        _ => false,
    }
}

/// Files that, sitting directly under the destination root, would change the
/// root folder's appearance in Windows Explorer (`desktop.ini` + the System
/// bit is what drives a custom folder icon). We never copy them from the
/// source root and never prune an existing one from the destination root —
/// the destination's identity belongs to the user, not to whatever happens
/// to be in the source.
fn is_root_icon_marker(rel_str: &str) -> bool {
    !rel_str.contains('/') && rel_str.eq_ignore_ascii_case("desktop.ini")
}

/// True if `rel_str` names a Windows folder-icon descriptor at *any* depth
/// in the source tree (basename `desktop.ini`, case-insensitive). These are
/// the bytes that pair with a parent's Readonly/System bit to render a
/// custom folder icon — the destination's icon state is therefore only as
/// trustworthy as a byte-identical copy of this file. We treat them
/// specially in two places:
///   1. The main copy loop never short-circuits a `desktop.ini` via the
///      size+mtime fast path — it always re-copies them.
///   2. A post-copy hash-verification pass compares src vs dst via xxh3 and
///      force re-copies any drift before the parent-folder attribute is
///      applied.
fn is_icon_descriptor(rel_str: &str) -> bool {
    std::path::Path::new(rel_str)
        .file_name()
        .map(|n| n.to_string_lossy().eq_ignore_ascii_case("desktop.ini"))
        .unwrap_or(false)
}

/// The relative paths the source says the destination should hold, with the
/// lookup rules the prune pass needs.
struct KeepSet {
    exact: HashSet<String>,
    /// Windows only: lowercased relative path -> the source's own spelling.
    #[cfg(windows)]
    by_lowercase: std::collections::HashMap<String, String>,
}

enum KeepStatus<'a> {
    /// Spelled the same on both sides — leave it alone.
    Exact,
    /// The source has this file but spells it differently in case only.
    /// Carries the source's spelling.
    CaseDrift(&'a str),
    /// Genuinely gone from the source.
    Absent,
}

impl KeepSet {
    fn new(rels: impl Iterator<Item = String>) -> Self {
        let exact: HashSet<String> = rels.collect();
        #[cfg(windows)]
        let by_lowercase = exact
            .iter()
            .map(|r| (r.to_lowercase(), r.clone()))
            .collect();
        Self {
            exact,
            #[cfg(windows)]
            by_lowercase,
        }
    }

    fn status(&self, rel: &str) -> KeepStatus<'_> {
        if self.exact.contains(rel) {
            return KeepStatus::Exact;
        }
        #[cfg(windows)]
        if let Some(source_rel) = self.by_lowercase.get(&rel.to_lowercase()) {
            return KeepStatus::CaseDrift(source_rel);
        }
        KeepStatus::Absent
    }
}

/// Everything the source walk learned, in one place.
struct WalkResult {
    files: Vec<FileEntry>,
    dirs: Vec<(PathBuf, String)>,
    total_bytes: u64,
    skipped: usize,
    /// Relative paths the user's exclude patterns matched.
    excluded: HashSet<String>,
    /// Relative paths we could not fully read: directories whose listing
    /// failed or was cut short, files we could not stat, entries whose type
    /// we could not determine. The prune pass must treat these exactly like
    /// exclusions — see `prune_destination`.
    unreadable: HashSet<String>,
}

/// Relative path of `path` under `root`, with `/` separators. Both walkers
/// key their sets on this representation, so it has to be computed the same
/// way in both places.
/// Whether failing to enumerate `dir` has to abort the whole walk.
///
/// For a subdirectory it does not: we record its relative path in
/// `unreadable` and prune leaves that subtree alone. The root has no such
/// escape hatch — `rel_of(root, root)` is `""`, which matches no destination
/// entry — so a root we could only list part-way is exactly as dangerous as
/// a root we could not open at all, and is treated the same way.
fn listing_failure_is_fatal(dir: &Path, root: &Path) -> bool {
    dir == root
}

fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

async fn walk(root: &Path, patterns: &glob::PatternSet) -> Result<WalkResult> {
    let mut files = Vec::new();
    let mut dirs: Vec<(PathBuf, String)> = Vec::new();
    let mut excluded: HashSet<String> = HashSet::new();
    let mut unreadable: HashSet<String> = HashSet::new();
    let mut total: u64 = 0;
    let mut skipped = 0usize;
    let root_canonical = long_path(root);
    let mut stack: Vec<PathBuf> = vec![root_canonical.clone()];

    while let Some(dir) = stack.pop() {
        let mut entries = match fs::read_dir(&dir).await {
            Ok(v) => v,
            Err(e) => {
                // The root itself being unreadable is not a partial walk, it
                // is no walk at all: continuing would hand prune an empty
                // `keep` set and wipe the whole destination.
                if dir == root_canonical {
                    return Err(anyhow!("Source folder could not be read: {}", e));
                }
                // A subtree we cannot enumerate. Record it so prune leaves
                // the destination copy alone instead of treating it as
                // deleted-at-source (which silently destroyed backups).
                warn!(dir = %dir.display(), "source directory could not be read: {}", e);
                unreadable.insert(rel_of(&root_canonical, &dir));
                skipped += 1;
                continue;
            }
        };
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    // Enumeration stopped early — the rest of this directory
                    // is unknown to us, so protect the whole directory. At
                    // the root there is nothing to protect it with, so this
                    // is fatal exactly like a root we could not open.
                    if listing_failure_is_fatal(&dir, &root_canonical) {
                        return Err(anyhow!("Source folder could not be listed: {}", e));
                    }
                    warn!(dir = %dir.display(), "source directory listing was cut short: {}", e);
                    unreadable.insert(rel_of(&root_canonical, &dir));
                    skipped += 1;
                    break;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(t) => t,
                Err(_) => {
                    unreadable.insert(rel_of(&root_canonical, &path));
                    skipped += 1;
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            let rel_str = rel_of(&root_canonical, &path);
            // Don't propagate a source-root `desktop.ini` to the destination
            // root — that would hijack the destination folder's icon. Treat
            // it as if it were excluded by user pattern, so prune leaves any
            // existing dest-root icon file alone too.
            if is_root_icon_marker(&rel_str) {
                excluded.insert(rel_str);
                continue;
            }
            if patterns.matches(&rel_str) {
                excluded.insert(rel_str);
                continue;
            }
            if file_type.is_dir() {
                dirs.push((path.clone(), rel_str));
                stack.push(path);
            } else if file_type.is_file() {
                let meta = match fs::metadata(&path).await {
                    Ok(m) => m,
                    Err(_) => {
                        // We can't compare size/mtime for a file we can't
                        // stat, so we won't copy it — which means prune must
                        // not read its absence from `keep` as "deleted".
                        unreadable.insert(rel_str);
                        skipped += 1;
                        continue;
                    }
                };
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                total += meta.len();
                files.push(FileEntry {
                    path,
                    rel: rel_str,
                    size: meta.len(),
                    mtime,
                });
            }
        }
    }
    Ok(WalkResult {
        files,
        dirs,
        total_bytes: total,
        skipped,
        excluded,
        unreadable,
    })
}

async fn copy_with_retries<F: FnMut(u64)>(
    src: &Path,
    dest: &Path,
    token: &CancellationToken,
    settings: &Settings,
    mut on_progress: F,
) -> Result<()> {
    let mut attempts = 0;
    let max = 3;
    loop {
        attempts += 1;
        on_progress(0); // reset per-file progress for any prior failed attempt
                        // std's remove_file already ignores READONLY on NTFS, but that fast
                        // path needs FileDispositionInfoEx — unsupported on the FAT32/exFAT
                        // volumes this app is most often pointed at. See fsutil::clear_readonly.
        clear_readonly(&long_path(dest));
        let _ = fs::remove_file(long_path(dest)).await;
        let res = copy_file(src, dest, token, settings, &mut on_progress).await;
        match res {
            Ok(()) => return Ok(()),
            Err(e) => {
                if token.is_cancelled() {
                    return Err(e);
                }
                if attempts >= max {
                    // Don't leave a half-written file in the destination after
                    // we give up — the next sync would either skip it on a
                    // size collision or re-copy redundantly (#12).
                    clear_readonly(&long_path(dest));
                    let _ = fs::remove_file(long_path(dest)).await;
                    return Err(e);
                }
                let backoff = 150u64 * (1u64 << (attempts - 1));
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            }
        }
    }
}

async fn copy_file<F: FnMut(u64)>(
    src: &Path,
    dest: &Path,
    token: &CancellationToken,
    settings: &Settings,
    mut on_progress: F,
) -> Result<()> {
    let src_l = long_path(src);
    let dest_l = long_path(dest);
    let src_meta = fs::metadata(&src_l).await.context("stat source")?;
    let mut reader = fs::File::open(&src_l).await.context("open source")?;
    // File::create is a hard PermissionDenied against an existing +R file,
    // so this must not be reached with the bit still set.
    clear_readonly(&dest_l);
    let mut writer = fs::File::create(&dest_l)
        .await
        .context("create destination")?;

    let buf_size = if src_meta.len() > 4 * 1024 * 1024 {
        1024 * 1024
    } else {
        256 * 1024
    };
    let mut buf = vec![0u8; buf_size];
    let mut file_so_far: u64 = 0;

    loop {
        tokio::select! {
            biased;
            _ = token.cancelled() => {
                drop(writer);
                let _ = fs::remove_file(&dest_l).await;
                return Err(anyhow!(CANCELLED_MSG));
            }
            read = reader.read(&mut buf) => {
                let n = read.context("read source")?;
                if n == 0 { break; }
                writer.write_all(&buf[..n]).await.context("write destination")?;
                file_so_far += n as u64;
                on_progress(file_so_far);
            }
        }
    }
    writer.flush().await?;
    // Durability: actually commit to disk before returning success.
    writer.sync_all().await.context("sync destination")?;
    drop(writer);

    if settings.preserve_mtime() {
        if let Ok(ft) = src_meta.modified() {
            let ft = FileTime::from_system_time(ft);
            let _ = filetime::set_file_mtime(&dest_l, ft);
        }
    }
    // Preserve Hidden / System / ReadOnly so things like `desktop.ini`
    // (which drives custom Windows folder icons) keep their attributes.
    if let Some(attrs) = read_attrs(src) {
        apply_attrs(dest, attrs);
    }
    Ok(())
}

async fn verify_files(
    files: &[FileEntry],
    backup_path: &Path,
    token: &CancellationToken,
) -> Result<()> {
    for f in files {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let dest = backup_path.join(&f.rel);
        let a = hash_file(&f.path).await?;
        let b = hash_file(&dest).await?;
        if a != b {
            return Err(anyhow!("Hash mismatch for {}", f.rel));
        }
    }
    Ok(())
}

async fn hash_file(path: &Path) -> Result<u64> {
    let mut f = fs::File::open(long_path(path)).await?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut hasher = Xxh3::new();
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.digest())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_descriptor_matches_desktop_ini_at_any_depth() {
        assert!(is_icon_descriptor("desktop.ini"));
        assert!(is_icon_descriptor("Desktop.INI"));
        assert!(is_icon_descriptor("Photos/desktop.ini"));
        assert!(is_icon_descriptor("a/b/c/Desktop.ini"));
        // Negatives
        assert!(!is_icon_descriptor("desktop.ini.bak"));
        assert!(!is_icon_descriptor("not-desktop.ini"));
        assert!(!is_icon_descriptor("desktop_ini"));
        assert!(!is_icon_descriptor("readme.txt"));
    }

    #[test]
    fn root_icon_marker_only_matches_root_desktop_ini() {
        assert!(is_root_icon_marker("desktop.ini"));
        assert!(is_root_icon_marker("Desktop.INI"));
        // Nested desktop.ini files are legitimate — they customize a real
        // sub-folder's icon and should round-trip.
        assert!(!is_root_icon_marker("sub/desktop.ini"));
        assert!(!is_root_icon_marker("a/b/desktop.ini"));
        assert!(!is_root_icon_marker("readme.txt"));
    }

    #[test]
    fn mtime_tolerates_2_seconds() {
        let base = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let plus_1 = base + std::time::Duration::from_secs(1);
        let plus_2 = base + std::time::Duration::from_secs(2);
        let plus_3 = base + std::time::Duration::from_secs(3);
        assert!(same_mtime(base, plus_1));
        assert!(same_mtime(base, plus_2));
        assert!(!same_mtime(base, plus_3));
    }

    /// Fresh, empty scratch directory for filesystem-touching tests.
    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("driveby-backup-test-{}", name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// The data-loss regression: a source subtree that `walk()` could not
    /// enumerate contributes nothing to `keep`, so prune used to read its
    /// destination copy as "deleted at source" and remove it — while the run
    /// still reported success. Anything named in `unreadable` must survive,
    /// contents included, and genuinely orphaned files must still go.
    #[tokio::test]
    async fn prune_preserves_unreadable_subtree() {
        let root = scratch("prune-unreadable");
        std::fs::create_dir_all(root.join("locked/deep")).unwrap();
        std::fs::write(root.join("locked/keep.txt"), b"precious").unwrap();
        std::fs::write(root.join("locked/deep/keep2.txt"), b"also precious").unwrap();
        std::fs::create_dir_all(root.join("gone")).unwrap();
        std::fs::write(root.join("gone/orphan.txt"), b"stale").unwrap();

        let keep = KeepSet::new(std::iter::empty());
        let excluded: HashSet<String> = HashSet::new();
        let unreadable: HashSet<String> = ["locked".to_string()].into_iter().collect();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();

        prune_destination(
            &root,
            &keep,
            &excluded,
            &unreadable,
            &glob::PatternSet::new(&[]),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        assert!(
            root.join("locked/keep.txt").exists(),
            "file under an unreadable source subtree was deleted"
        );
        assert!(
            root.join("locked/deep/keep2.txt").exists(),
            "prune recursed into an unreadable subtree"
        );
        assert!(
            !root.join("gone/orphan.txt").exists(),
            "genuinely orphaned file should still be pruned"
        );
        assert_eq!(stats.deleted, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A custom-icon folder carries `+R` by construction — that bit is half
    /// of what makes Explorer render the icon, and apply_attrs mirrors it
    /// onto the destination. `remove_dir` is a hard PermissionDenied against
    /// such a folder, so once its contents left the source the emptied
    /// directory stayed in the destination forever.
    #[cfg(windows)]
    #[tokio::test]
    async fn prune_removes_emptied_readonly_directory() {
        let root = scratch("prune-readonly-dir");
        let dir = root.join("iconfolder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("orphan.txt"), b"stale").unwrap();
        apply_attrs(&dir, 0x1); // what a custom-icon folder looks like

        let empty: HashSet<String> = HashSet::new();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();
        prune_destination(
            &root,
            &KeepSet::new(std::iter::empty()),
            &empty,
            &empty,
            &glob::PatternSet::new(&[]),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        assert_eq!(stats.deleted, 1);
        assert!(
            !dir.exists(),
            "emptied read-only directory was left behind by prune"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// prune stripped `+R` from every directory it walked, not just the ones
    /// it was about to remove, and only `mirror_dir_attrs_phase` — two
    /// phases later — put the bit back. `verify_icons_phase` sits in between
    /// and can return early on cancellation, so hitting Stop at the wrong
    /// moment left every custom-icon folder in the destination rendering
    /// with the default icon until a later run completed all its phases.
    #[cfg(windows)]
    #[tokio::test]
    async fn prune_leaves_a_surviving_icon_folder_readonly() {
        let root = scratch("prune-keeps-readonly");
        let dir = root.join("iconfolder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), b"kept").unwrap();
        apply_attrs(&dir, 0x1); // what a custom-icon folder looks like

        let empty: HashSet<String> = HashSet::new();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();
        prune_destination(
            &root,
            &KeepSet::new(["iconfolder/keep.txt".to_string()].into_iter()),
            &empty,
            &empty,
            &glob::PatternSet::new(&[]),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        assert_eq!(stats.deleted, 0);
        assert!(dir.exists(), "a folder with a kept file must survive prune");
        assert_eq!(
            read_attrs(&dir).unwrap() & 0x1,
            0x1,
            "prune cleared +R from a directory it did not remove"
        );

        clear_readonly(&dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The user re-cases a source file (`readme.md` -> `README.md`). NTFS is
    /// case-preserving, so the copy loop writes through the existing entry
    /// and the destination keeps the old spelling. prune used to miss that
    /// spelling in `keep` and delete the file it had just copied, leaving
    /// the backup incomplete for a whole run while still reporting success.
    #[cfg(windows)]
    #[tokio::test]
    async fn prune_recases_instead_of_deleting_on_case_only_rename() {
        let root = scratch("prune-case-drift");
        std::fs::write(root.join("Readme.md"), b"content").unwrap();
        std::fs::write(root.join("dropped.txt"), b"stale").unwrap();

        let keep = KeepSet::new(["README.md".to_string()].into_iter());
        let empty: HashSet<String> = HashSet::new();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();
        prune_destination(
            &root,
            &keep,
            &empty,
            &empty,
            &glob::PatternSet::new(&[]),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        // exists() is case-insensitive on Windows, so check the actual
        // directory entry to prove the spelling was corrected.
        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["README.md".to_string()]);
        assert_eq!(std::fs::read(root.join("README.md")).unwrap(), b"content");
        assert_eq!(stats.recased, 1);
        assert_eq!(stats.deleted, 1, "the genuinely orphaned file should go");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Copying over a read-only destination file must work: `File::create`
    /// is a hard PermissionDenied against one. On NTFS the preceding
    /// `remove_file` already covers this (std deletes read-only files), so
    /// this test only fails on filesystems without FileDispositionInfoEx —
    /// it stands as a guard against removing either clear_readonly call.
    #[cfg(windows)]
    #[tokio::test]
    async fn copy_with_retries_overwrites_readonly_destination() {
        let root = scratch("readonly-dest");
        let src = root.join("src.txt");
        let dest = root.join("dest.txt");
        std::fs::write(&src, b"new content").unwrap();
        std::fs::write(&dest, b"stale").unwrap();
        apply_attrs(&dest, 0x1);

        let token = CancellationToken::new();
        let settings = Settings::default();
        copy_with_retries(&src, &dest, &token, &settings, |_| {})
            .await
            .expect("read-only destination must be overwritable");

        assert_eq!(std::fs::read(&dest).unwrap(), b"new content");
        clear_readonly(&dest);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The root of the source being unreadable is not a partial walk, it is
    /// no walk at all — returning an empty file list would hand prune an
    /// empty `keep` set and wipe the entire destination.
    #[tokio::test]
    async fn walk_errors_when_root_is_missing() {
        let missing = std::env::temp_dir().join("driveby-backup-test-does-not-exist");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(walk(&missing, &glob::PatternSet::new(&[])).await.is_err());
    }

    /// `read_dir` on the root failing is fatal, but the sibling case — the
    /// listing being cut short part-way through — used to fall into the
    /// generic "record it and carry on" branch. There is nothing to record
    /// it *with*: `rel_of(root, root)` is `""`, which matches no destination
    /// entry, so the whole destination went unprotected.
    #[test]
    fn a_cut_short_listing_is_only_fatal_at_the_root() {
        let root = Path::new("C:/src");
        assert!(listing_failure_is_fatal(root, root));
        assert!(!listing_failure_is_fatal(&root.join("sub"), root));
    }

    /// Defence in depth for the same bug: whatever put an empty relative
    /// path into `unreadable`, it means "we do not know what the source root
    /// holds" — which has to protect the whole destination, not nothing.
    #[tokio::test]
    async fn prune_is_a_no_op_when_the_root_itself_is_unreadable() {
        let root = scratch("prune-unreadable-root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/keep.txt"), b"precious").unwrap();
        std::fs::write(root.join("top.txt"), b"also precious").unwrap();

        let empty: HashSet<String> = HashSet::new();
        let unreadable: HashSet<String> = [String::new()].into_iter().collect();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();

        prune_destination(
            &root,
            &KeepSet::new(std::iter::empty()),
            &empty,
            &unreadable,
            &glob::PatternSet::new(&[]),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        assert!(
            root.join("top.txt").exists(),
            "prune emptied the destination after an unreadable source root"
        );
        assert!(root.join("sub/keep.txt").exists());
        assert_eq!(stats.deleted, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
