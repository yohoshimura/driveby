use crate::fsutil::{
    apply_attrs, clear_readonly, long_path, path_contains, read_attrs, reject_overlap,
    scratch_path, ATTR_KEEP,
};
use crate::glob;
use crate::persist;
use crate::ratelimit;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use filetime::FileTime;
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager, Runtime};
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
    /// The shape every tasks.json written before 1.7.2 holds: exactly one
    /// destination. The frontend rewrites the file into `destinations` on
    /// first load, but this side has to keep reading it — the scheduler
    /// deserialises the same struct and can tick before that migration has
    /// run, and a user who downgrades writes the old shape back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destinations: Option<Vec<String>>,
    #[serde(default)]
    pub schedule: Option<String>,
    /// Days of the week a `custom` schedule runs on, 0 = Sunday — the
    /// numbering `Date#getDay` uses, because the form is what writes them.
    #[serde(default, rename = "scheduleDays")]
    pub schedule_days: Option<Vec<u8>>,
    /// Local time of day a `custom` schedule runs at, "HH:MM".
    #[serde(default, rename = "scheduleTime")]
    pub schedule_time: Option<String>,
    #[serde(default, rename = "lastBackup")]
    pub last_backup: Option<String>,
}

impl Task {
    /// Every destination this task writes to, in the order the user listed
    /// them, blanks dropped and exact repeats collapsed.
    ///
    /// Only exact repeats: two spellings of the same folder (a trailing
    /// separator, a different case) are left in, because deciding they are
    /// the same folder means touching the filesystem — that is
    /// `reject_destination_overlaps`' job, and it has to run anyway.
    pub fn destinations(&self) -> Vec<String> {
        let listed = match &self.destinations {
            Some(list) if !list.is_empty() => list.clone(),
            // `into_iter` on the Option yields nothing when it is None, so a
            // task carrying neither field simply has no destinations.
            _ => self.destination.clone().into_iter().collect(),
        };
        let mut seen = HashSet::new();
        listed
            .into_iter()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .filter(|d| seen.insert(d.clone()))
            .collect()
    }
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
    #[serde(default, rename = "parallelCopies")]
    pub parallel_copies: Option<u32>,
    #[serde(default, rename = "maxSpeedMbps")]
    pub max_speed_mbps: Option<f64>,
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
    /// How many files the copy loop keeps in flight. 1 is the escape hatch
    /// that reproduces the historical sequential behavior exactly (spinning
    /// disks can prefer it); the cap keeps us from turning a USB drive into
    /// a seek storm.
    fn parallel_copies(&self) -> usize {
        self.parallel_copies.unwrap_or(4).clamp(1, 8) as usize
    }
    /// The copy ceiling in bytes per second, 0 meaning none.
    ///
    /// The setting is in MiB/s: the UI says "MB" (English) and "Mo"
    /// (French), and everywhere else in the app those already mean 1024²
    /// bytes — `formatBytes` divides by 1024. A number that reads 50 in
    /// Settings and 50 in the transfer speed beside it has to mean the same
    /// thing in both places.
    ///
    /// A missing, zero, negative or NaN value is "no ceiling"; the `as`
    /// cast saturates, so an absurd number is simply a very high ceiling
    /// rather than a wrapped-around tiny one.
    fn max_speed_bytes(&self) -> u64 {
        match self.max_speed_mbps {
            Some(mbps) if mbps > 0.0 => (mbps * 1024.0 * 1024.0) as u64,
            _ => 0,
        }
    }
}

/// One registered run: the token that stops it, plus a serial number so a
/// run finishing late can only ever remove its *own* registration.
struct Registration {
    run: u64,
    token: CancellationToken,
}

#[derive(Default)]
pub struct BackupState {
    active: Arc<DashMap<String, Registration>>,
    next_run: Arc<AtomicU64>,
}

impl BackupState {
    /// Cancel the run without freeing its slot. Cancelling only trips the
    /// token; the run then still has to drain its in-flight copies, which
    /// can take seconds on a slow target. Releasing the slot here would let
    /// a second run start against the same destination while the first is
    /// still writing, and the two would delete each other's files. The slot
    /// stays taken until the run itself unregisters — which is exactly how
    /// RestoreState::cancel has always behaved (#R3).
    pub fn cancel(&self, task_id: &str) {
        // Clone out and drop the map guard before cancelling, so no shard
        // lock is held across the wakeups that cancel() triggers.
        let token = self.active.get(task_id).map(|reg| reg.token.clone());
        if let Some(token) = token {
            token.cancel();
        }
    }
    pub fn is_active(&self, task_id: &str) -> bool {
        self.active.contains_key(task_id)
    }
    /// Atomic register-if-absent. Returns None when a backup is already
    /// running for this task — the caller must not start a second one.
    fn try_register(&self, task_id: &str) -> Option<(u64, CancellationToken)> {
        let token = CancellationToken::new();
        let run = self.next_run.fetch_add(1, Ordering::Relaxed);
        // DashMap::entry gives us a single locked slot; insert only if vacant.
        match self.active.entry(task_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(_) => None,
            dashmap::mapref::entry::Entry::Vacant(v) => {
                v.insert(Registration {
                    run,
                    token: token.clone(),
                });
                Some((run, token))
            }
        }
    }
    /// Remove only our own registration. A run that finishes after a newer
    /// one has started must not evict the newcomer's token — that would
    /// leave the new run uncancellable and the slot open for a third.
    fn unregister(&self, task_id: &str, run: u64) {
        self.active.remove_if(task_id, |_, reg| reg.run == run);
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
    /// Which destination these numbers belong to. Destinations are written
    /// one after another under a single task id, so without this the UI
    /// would see the progress bar restart at zero with nothing to explain
    /// why.
    destination: String,
    dest_index: u32,
    dest_count: u32,
}

/// How one destination of a run ended.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DestinationStatus {
    Success,
    Error,
    Cancelled,
    /// The folder was not there to write to — an unplugged drive, a network
    /// share that is down. Kept apart from `Error` because it is the one
    /// failure the user can fix by plugging something in, and the one we
    /// deliberately do not turn into a red history row every 24 hours.
    Unreachable,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DestinationOutcome {
    pub path: String,
    pub status: DestinationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<u64>,
}

impl DestinationOutcome {
    /// A destination that never ran: no counts, just why.
    fn stillborn(path: &Path, status: DestinationStatus, error: Option<String>) -> Self {
        Self {
            path: path.to_string_lossy().to_string(),
            status,
            error,
            total_bytes: None,
            total_files: None,
            duration_ms: None,
            skipped: None,
            cleaned: None,
            unchanged: None,
            failed: None,
            verified: None,
            unreadable: None,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompletePayload {
    pub backup_id: String,
    pub task_id: String,
    pub success: bool,
    /// At least one destination was written and at least one was not.
    /// `success` stays false with it: the copy the user asked for is not
    /// everywhere it was supposed to be, and `lastBackup` must not move.
    pub partial: bool,
    pub cancelled: bool,
    /// One entry per destination the run considered, in task order. The
    /// scalar fields below are this list folded up — `path` is the first
    /// destination that succeeded, the counts are sums.
    pub destinations: Vec<DestinationOutcome>,
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

pub(crate) struct FileEntry {
    pub(crate) path: PathBuf,
    pub(crate) rel: String,
    pub(crate) size: u64,
    pub(crate) mtime: SystemTime,
}

/// Cooperative-cancellation sentinel string used by the inner `execute_one()`
/// pipeline to short-circuit on `CancellationToken::is_cancelled()`. The
/// outer `run_backup()` no longer string-matches this — it consults the
/// token directly — but having a single named constant keeps the in-function
/// returns consistent and greppable.
pub(crate) const CANCELLED_MSG: &str = "backup cancelled";

// ─────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────

/// Generic over the Tauri runtime purely so tests can drive the real
/// pipeline against `tauri::test::mock_app()`. Production passes an
/// `AppHandle` (i.e. `AppHandle<Wry>`) and infers `R` from it, so no call
/// site changes.
pub async fn run_backup<R: Runtime>(
    app: &AppHandle<R>,
    state: &BackupState,
    task: Task,
    settings: Settings,
) -> Result<CompletePayload> {
    let backup_id = uuid::Uuid::new_v4().to_string();

    // Atomic guard: refuse a second concurrent run for the same task. Without
    // this, double-click or scheduler-while-manual would race two writers
    // against the same destination tree (#1).
    let (run, token) = match state.try_register(&task.id) {
        Some(t) => t,
        None => {
            return Err(anyhow!("A backup is already running for this task"));
        }
    };

    // The ceiling is process-wide, so the newest run's setting is the one in
    // force — including for a run already under way, which is what makes
    // lowering it while something is copying take effect at the next chunk
    // instead of at the next backup.
    ratelimit::shared().set_rate(settings.max_speed_bytes());

    let result = execute_all(app, &backup_id, &task, &settings, &token).await;

    // Snapshot cancellation state from the token *before* unregister, so we
    // don't depend on stringly-typed error matching (`err.to_string().contains
    // ("ABORTED")`). The token is the source of truth: a successful run
    // always returns Ok, a cancelled one always has the token tripped, and
    // any other failure leaves the token alone.
    let cancelled = token.is_cancelled();
    state.unregister(&task.id, run);

    let payload = match result {
        // A cancellation landing after the last checkpoint still lets
        // execute_one() return Ok. Taking that at face value would report a
        // clean success and stamp lastBackup on a run the user stopped, so
        // the token decides here too — mirroring restore::conclude (#R4).
        Ok(p) => CompletePayload {
            success: p.success && !cancelled,
            cancelled,
            ..p
        },
        // Nothing ran at all: a missing source, no destination set, or two
        // destinations nested in one another. There is no per-destination
        // detail to report because no destination was ever opened.
        Err(err) => CompletePayload {
            backup_id: backup_id.clone(),
            task_id: task.id.clone(),
            success: false,
            partial: false,
            cancelled,
            destinations: Vec::new(),
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

async fn update_last_backup<R: Runtime>(app: &AppHandle<R>, task_id: &str) -> Result<()> {
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
struct RunCtx<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    backup_id: &'a str,
    task_id: &'a str,
    target: &'a Path,
    settings: &'a Settings,
    token: &'a CancellationToken,
    started: Instant,
    total_bytes: u64,
    total_files: u64,
    /// Position of `target` in the task's destination list, and how many
    /// there are. Carried into every progress event so the UI can say
    /// "2 of 3" rather than show a bar that mysteriously starts over.
    dest_index: u32,
    dest_count: u32,
}

impl<R: Runtime> RunCtx<'_, R> {
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
                destination: self.target.to_string_lossy().to_string(),
                dest_index: self.dest_index,
                dest_count: self.dest_count,
            },
        );
    }

    /// Throttled progress emit for the copy loop: at most one event per
    /// 100ms across all in-flight workers, with speed and ETA derived from
    /// elapsed wall time. `try_lock` keeps workers from queueing behind the
    /// throttle — a contended tick is simply skipped and the next chunk
    /// emits instead.
    fn maybe_emit(&self, live: &LiveProgress, phase: &'static str) {
        let Ok(mut last_emit) = live.last_emit.try_lock() else {
            return;
        };
        if last_emit.elapsed().as_millis() < 100 {
            return;
        }
        *last_emit = Instant::now();
        let copied_bytes = live.bytes.load(Ordering::Relaxed);
        let copied_files = live.files.load(Ordering::Relaxed);
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
                destination: self.target.to_string_lossy().to_string(),
                dest_index: self.dest_index,
                dest_count: self.dest_count,
            },
        );
    }
}

/// Shared live progress for the concurrent copy loop. Workers add byte
/// deltas as chunks land and roll their own bytes back out when an attempt
/// fails or is cancelled, so the global count stays monotone-accurate
/// without a coordinator task.
struct LiveProgress {
    bytes: AtomicU64,
    files: AtomicU64,
    last_emit: std::sync::Mutex<Instant>,
}

impl LiveProgress {
    fn new() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            files: AtomicU64::new(0),
            last_emit: std::sync::Mutex::new(Instant::now()),
        }
    }
    /// Positive deltas are streamed chunks; a negative delta is a worker
    /// taking a failed attempt's bytes back out.
    fn on_delta(&self, delta: i64) {
        if delta >= 0 {
            self.bytes.fetch_add(delta as u64, Ordering::Relaxed);
        } else {
            self.bytes.fetch_sub(delta.unsigned_abs(), Ordering::Relaxed);
        }
    }
}

/// Running totals threaded through the phases and folded into the final
/// payload. One accumulator rather than a dozen `let mut` bindings living
/// for the whole length of one destination.
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

/// Validate the source, once for the whole run. Fatal on failure: with no
/// readable source there is nothing to write to any destination.
pub(crate) async fn preflight_source(task: &Task) -> Result<PathBuf> {
    let source = PathBuf::from(&task.source);
    if !source.is_absolute() {
        return Err(anyhow!("Paths must be absolute"));
    }
    let src_meta = fs::metadata(long_path(&source))
        .await
        .map_err(|_| anyhow!("Source folder not found"))?;
    if !src_meta.is_dir() {
        return Err(anyhow!("Source is not a directory"));
    }
    Ok(source)
}

/// Validate one destination. Deliberately *not* fatal to the run: a task
/// can name three drives with only two plugged in, and those two should
/// still get their backup. The caller turns an error here into an
/// `Unreachable` outcome for this destination alone.
async fn preflight_destination(destination: &Path) -> Result<()> {
    if !destination.is_absolute() {
        return Err(anyhow!("Paths must be absolute"));
    }
    let dest_meta = fs::metadata(long_path(destination))
        .await
        .map_err(|_| anyhow!("Destination not found"))?;
    if !dest_meta.is_dir() {
        return Err(anyhow!("Destination is not a directory"));
    }
    Ok(())
}

/// Reject every nesting the run cannot survive, before a single byte moves.
///
/// Source against destination is the long-standing guard (#3), shared with
/// restore. Destination against destination arrives with multiple
/// destinations and is just as destructive: each destination is mirror-
/// pruned against the *source*, so a destination nested inside another is
/// absent from its host's keep set and the host's prune pass deletes the
/// whole subtree — one backup erasing another.
///
/// Fatal for the entire run rather than per destination, because the damage
/// would be done by the destination that looks perfectly healthy.
pub(crate) fn reject_destination_overlaps(source: &Path, destinations: &[PathBuf]) -> Result<()> {
    for (i, dest) in destinations.iter().enumerate() {
        reject_overlap(source, dest)?;
        for other in &destinations[i + 1..] {
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
    Ok(())
}

/// Outcome of one file's trip through the copy loop, folded into
/// `PhaseStats` by the single consumer of the worker stream — workers
/// return values, so the stats stay single-owner.
enum FileOutcome {
    Copied { rel: String, hash: u64 },
    Unchanged,
    Failed { rel: String, error: anyhow::Error },
    /// The run was cancelled or aborted while (or before) this file was in
    /// flight — nothing to count.
    Aborted,
}

/// Mirror the source files into the destination, `parallelCopies` files at
/// a time. Files already present with matching size + mtime (within the 2s
/// tolerance) are skipped. Returns the copy-time xxh3 of every file
/// actually copied — what the verify phase later checks the destination
/// against.
///
/// Concurrency notes: a bounded window of borrowed futures — not spawned
/// tasks — because tokio's fs ops already run on the blocking pool, so
/// concurrent futures on this one task get real I/O overlap without
/// `'static` bounds. Platform-native fast copies (CopyFile2 /
/// clonefile / copy_file_range) were considered and rejected for now: each
/// breaks hash-during-copy (forcing verify back to re-reading the source),
/// carries its own partial-file and cancellation semantics, and none of it
/// is exercised by CI off Windows yet.
async fn copy_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
    files: &[FileEntry],
    stats: &mut PhaseStats,
) -> Result<Vec<(String, u64)>> {
    let live = LiveProgress::new();
    // A child token lets a fatal error (continueOnError=false) stop the
    // other workers without tripping the parent — run_backup reads the
    // parent afterwards to tell user cancellation apart from an abort.
    let abort = ctx.token.child_token();

    let mut hashes: Vec<(String, u64)> = Vec::new();
    let mut first_error: Option<anyhow::Error> = None;

    // Manual window over FuturesUnordered rather than buffer_unordered —
    // same bounded concurrency, no closure, and therefore none of the
    // higher-ranked lifetime trouble closures cause in a Send future.
    let mut pending = files.iter();
    let mut in_flight = FuturesUnordered::new();
    loop {
        while in_flight.len() < ctx.settings.parallel_copies() {
            match pending.next() {
                Some(file) => in_flight.push(copy_one(ctx, file, &live, &abort)),
                None => break,
            }
        }
        let Some(outcome) = in_flight.next().await else {
            break;
        };
        match outcome {
            FileOutcome::Copied { rel, hash } => hashes.push((rel, hash)),
            FileOutcome::Unchanged => stats.unchanged += 1,
            FileOutcome::Failed { rel, error } => {
                stats.failed += 1;
                warn!(target = %rel, "copy failed: {}", error);
                stats.errors.push(format!("{}: {}", rel, error));
                if !ctx.settings.continue_on_error() && first_error.is_none() {
                    first_error = Some(error);
                    abort.cancel();
                }
            }
            FileOutcome::Aborted => {}
        }
    }

    stats.copied_bytes = live.bytes.load(Ordering::Relaxed);
    stats.copied_files = live.files.load(Ordering::Relaxed);

    ctx.check_cancelled()?;
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(hashes)
}

/// One file through the copy loop: ensure the parent, skip if unchanged,
/// otherwise copy with retries. Runs concurrently with its siblings — all
/// shared mutation goes through `live`'s atomics.
async fn copy_one<R: Runtime>(
    ctx: &RunCtx<'_, R>,
    file: &FileEntry,
    live: &LiveProgress,
    abort: &CancellationToken,
) -> FileOutcome {
    if abort.is_cancelled() {
        return FileOutcome::Aborted;
    }
    let dest_path = ctx.target.join(&file.rel);
    if let Some(parent) = dest_path.parent() {
        // Don't kill the whole job because one parent can't be made (#8).
        if let Err(e) = fs::create_dir_all(long_path(parent)).await {
            return FileOutcome::Failed {
                rel: file.rel.clone(),
                error: anyhow!("create parent: {}", e),
            };
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
        live.bytes.fetch_add(file.size, Ordering::Relaxed);
        live.files.fetch_add(1, Ordering::Relaxed);
        ctx.maybe_emit(live, "syncing");
        return FileOutcome::Unchanged;
    }

    let result = copy_with_retries(&file.path, &dest_path, abort, ctx.settings, &mut |delta| {
        live.on_delta(delta);
        if delta > 0 {
            ctx.maybe_emit(live, "copying");
        }
    })
    .await;

    match result {
        Ok(hash) => {
            live.files.fetch_add(1, Ordering::Relaxed);
            FileOutcome::Copied {
                rel: file.rel.clone(),
                hash,
            }
        }
        Err(error) => {
            if abort.is_cancelled() {
                FileOutcome::Aborted
            } else {
                FileOutcome::Failed {
                    rel: file.rel.clone(),
                    error,
                }
            }
        }
    }
}

/// Re-spell destination directories whose on-disk casing no longer matches
/// the source: NTFS is case-insensitive but case-preserving, so when the
/// user re-cases a folder at source, every write goes through the existing
/// destination entry and the old spelling survives. File-level drift is
/// `recase_entry`'s job during prune; this pass owns the directory
/// components, which `with_file_name` can never reach.
///
/// Shallow-first so parents are corrected before children are looked at —
/// though lookups resolve case-insensitively either way, so the pass is
/// order-tolerant, and it is idempotent: a cancelled run finishes the job
/// on the next one.
// TODO(macOS): APFS is case-insensitive by default too — extend this cfg
// (and KeepSet::by_lowercase, and fsutil::on_disk_name) once macOS is a
// supported target.
#[cfg(windows)]
async fn recase_dirs_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
    dirs: &[(PathBuf, String)],
    stats: &mut PhaseStats,
) {
    let mut rels: Vec<&str> = dirs.iter().map(|(_, r)| r.as_str()).collect();
    rels.sort_by_key(|r| r.matches('/').count());
    for rel in rels {
        if ctx.token.is_cancelled() {
            return;
        }
        let Some(want) = Path::new(rel).file_name() else {
            continue;
        };
        let dest = ctx.target.join(rel);
        // Not created yet (empty source dirs only materialize in
        // mirror_dir_attrs_phase) — nothing to re-spell.
        let Some(actual) = crate::fsutil::on_disk_name(&dest) else {
            continue;
        };
        if actual == want {
            continue;
        }
        // The lookup found the entry under a different spelling. Only act
        // when the difference is case-only: an 8.3 short-name alias could
        // otherwise match a genuinely different name, and renaming that
        // would not be a recase.
        if actual.to_string_lossy().to_lowercase() != want.to_string_lossy().to_lowercase() {
            continue;
        }
        let parent = dest.parent().unwrap_or(ctx.target);
        let old = parent.join(&actual);
        match fs::rename(long_path(&old), long_path(&dest)).await {
            Ok(()) => stats.recased += 1,
            Err(e) => warn!("could not restore source casing for directory {}: {}", rel, e),
        }
    }
}

/// Mirror-delete pass: remove anything in the destination that is no longer
/// in the source. Excluded paths are *preserved* — exclude means "don't
/// copy", not "delete from dest" (#2) — as are paths the walk could not read.
async fn prune_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
    files: &[FileEntry],
    protected: &ProtectedSet<'_>,
    stats: &mut PhaseStats,
) -> Result<()> {
    ctx.emit_phase("pruning", stats.copied_bytes, stats.copied_files, None);
    let keep = KeepSet::new(files.iter().map(|f| f.rel.clone()));
    if let Err(e) = prune_destination(ctx.target, &keep, protected, ctx.token, stats).await {
        ctx.check_cancelled()?;
        warn!("prune destination failed: {}", e);
    }
    if stats.recased > 0 {
        info!(
            "restored source casing on {} destination path(s)",
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
async fn verify_icons_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
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
            match copy_with_retries(&f.path, &dest_path, ctx.token, ctx.settings, &mut |_| {})
                .await
            {
                Ok(_) => stats.icon_resyncs += 1,
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
async fn mirror_dir_attrs_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
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

/// Optional hash verification of this run's copies. Only runs on a clean
/// copy pass: re-reading files we already know failed would just report the
/// failure a second time, more slowly. The source side's hash was captured
/// while copying, so only the destination is re-read — and files skipped as
/// unchanged were verified by the run that originally copied them.
async fn verify_phase<R: Runtime>(
    ctx: &RunCtx<'_, R>,
    hashes: &[(String, u64)],
    stats: &PhaseStats,
) -> Result<bool> {
    if !ctx.settings.verify() || stats.failed != 0 {
        return Ok(false);
    }
    ctx.emit_phase("verifying", stats.copied_bytes, stats.copied_files, None);
    verify_files(hashes, ctx.target, ctx.token).await?;
    Ok(true)
}

/// Run the task against each of its destinations, one after another.
///
/// Sequential, not concurrent. Two destinations in flight would read the
/// source twice at once — a seek storm on a spinning disk — and could not
/// share one read stream anyway, because whether a given file needs copying
/// is a question each destination answers for itself.
///
/// The source walk *is* shared: it depends only on the source and the
/// exclude patterns, so walking once per destination would repeat the same
/// traversal for nothing. It also means the three copies are made from one
/// snapshot of the tree rather than three snapshots taken minutes apart,
/// which is rather the point of writing to three places.
async fn execute_all<R: Runtime>(
    app: &AppHandle<R>,
    backup_id: &str,
    task: &Task,
    settings: &Settings,
    token: &CancellationToken,
) -> Result<CompletePayload> {
    let started = Instant::now();
    let source = preflight_source(task).await?;
    let destinations: Vec<PathBuf> = task.destinations().iter().map(PathBuf::from).collect();
    if destinations.is_empty() {
        return Err(anyhow!("No destination set for this task"));
    }
    reject_destination_overlaps(&source, &destinations)?;

    // One `backup-started` per run, not per destination: the UI opens a
    // single progress slot keyed by task id, and which destination is being
    // written rides along on the progress events.
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

    // Built once from the walk: what prune must leave alone is a property
    // of the source side, identical for every destination.
    let protected = ProtectedSet::new(&walked, &patterns);

    let dest_count = destinations.len() as u32;
    let mut outcomes: Vec<DestinationOutcome> = Vec::with_capacity(destinations.len());
    for (index, destination) in destinations.iter().enumerate() {
        // Stopping mid-run leaves the destinations we never reached marked
        // cancelled rather than silently absent from the report.
        if token.is_cancelled() {
            outcomes.push(DestinationOutcome::stillborn(
                destination,
                DestinationStatus::Cancelled,
                None,
            ));
            continue;
        }
        if let Err(e) = preflight_destination(destination).await {
            warn!(dest = %destination.display(), "skipping destination: {}", e);
            outcomes.push(DestinationOutcome::stillborn(
                destination,
                DestinationStatus::Unreachable,
                Some(e.to_string()),
            ));
            continue;
        }
        let outcome = execute_one(
            app,
            backup_id,
            task,
            destination,
            index as u32,
            dest_count,
            &mut walked,
            &protected,
            settings,
            token,
        )
        .await;
        outcomes.push(outcome.unwrap_or_else(|e| {
            let cancelled = token.is_cancelled();
            DestinationOutcome::stillborn(
                destination,
                if cancelled {
                    DestinationStatus::Cancelled
                } else {
                    DestinationStatus::Error
                },
                if cancelled {
                    None
                } else {
                    Some(e.to_string())
                },
            )
        }));
    }

    Ok(fold_outcomes(backup_id, task, started, outcomes))
}

/// Mirror the walked source into one destination.
#[allow(clippy::too_many_arguments)]
async fn execute_one<R: Runtime>(
    app: &AppHandle<R>,
    backup_id: &str,
    task: &Task,
    destination: &Path,
    dest_index: u32,
    dest_count: u32,
    walked: &mut WalkResult,
    protected: &ProtectedSet<'_>,
    settings: &Settings,
    token: &CancellationToken,
) -> Result<DestinationOutcome> {
    let ctx = RunCtx {
        app,
        backup_id,
        task_id: &task.id,
        target: destination,
        settings,
        token,
        started: Instant::now(),
        total_bytes: walked.total_bytes,
        total_files: walked.files.len() as u64,
        dest_index,
        dest_count,
    };
    let mut stats = PhaseStats::default();

    let hashes = copy_phase(&ctx, &walked.files, &mut stats).await?;
    #[cfg(windows)]
    recase_dirs_phase(&ctx, &walked.dirs, &mut stats).await;
    prune_phase(&ctx, &walked.files, protected, &mut stats).await?;
    verify_icons_phase(&ctx, &walked.files, &mut stats).await?;
    mirror_dir_attrs_phase(&ctx, &mut walked.dirs, &mut stats).await;

    // Force a final 100% emit so the UI reflects completion even when the
    // throttle would have skipped the last chunk.
    ctx.emit_phase("finishing", ctx.total_bytes, ctx.total_files, Some(0));

    let verified = verify_phase(&ctx, &hashes, &stats).await?;

    for e in &stats.errors {
        warn!("file error: {}", e);
    }

    Ok(DestinationOutcome {
        path: destination.to_string_lossy().to_string(),
        status: if stats.failed == 0 {
            DestinationStatus::Success
        } else {
            DestinationStatus::Error
        },
        error: if stats.failed > 0 {
            Some(format!("{} file(s) failed", stats.failed))
        } else {
            None
        },
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

/// Fold the per-destination outcomes into the run-level payload the toast,
/// the notification and the history row all read.
///
/// The scalar counts are sums across the destinations that actually ran:
/// three copies of a 4 GB source really did move 12 GB, and a history row
/// claiming 4 GB would be describing a third of the work.
fn fold_outcomes(
    backup_id: &str,
    task: &Task,
    started: Instant,
    destinations: Vec<DestinationOutcome>,
) -> CompletePayload {
    let succeeded: Vec<&DestinationOutcome> = destinations
        .iter()
        .filter(|d| d.status == DestinationStatus::Success)
        .collect();
    let success = succeeded.len() == destinations.len() && !succeeded.is_empty();
    let partial = !succeeded.is_empty() && succeeded.len() < destinations.len();
    let first_path = succeeded.first().map(|d| d.path.clone());
    let verified = !succeeded.is_empty() && succeeded.iter().all(|d| d.verified == Some(true));

    // A cancelled destination is not an error to report — the user asked
    // for it. Only genuine failures and absent drives make it into the
    // message, so a stopped run doesn't display a list of paths.
    let faults: Vec<String> = destinations
        .iter()
        .filter(|d| {
            matches!(
                d.status,
                DestinationStatus::Error | DestinationStatus::Unreachable
            )
        })
        .map(|d| match &d.error {
            Some(e) => format!("{}: {}", d.path, e),
            None => d.path.clone(),
        })
        .collect();

    let total_bytes = destinations.iter().filter_map(|d| d.total_bytes).sum();
    let total_files = destinations.iter().filter_map(|d| d.total_files).sum();
    let skipped = destinations.iter().filter_map(|d| d.skipped).sum();
    let cleaned = destinations.iter().filter_map(|d| d.cleaned).sum();
    let unchanged = destinations.iter().filter_map(|d| d.unchanged).sum();
    let failed = destinations.iter().filter_map(|d| d.failed).sum();
    let unreadable = destinations.iter().filter_map(|d| d.unreadable).sum();
    // Everything above borrows `destinations`; the payload owns it, so the
    // borrows have to be finished before the move.
    drop(succeeded);

    CompletePayload {
        backup_id: backup_id.to_string(),
        task_id: task.id.clone(),
        success,
        partial,
        cancelled: false,
        error: if faults.is_empty() {
            None
        } else {
            Some(faults.join("; "))
        },
        path: first_path,
        total_bytes: Some(total_bytes),
        total_files: Some(total_files),
        duration_ms: Some(started.elapsed().as_millis() as u64),
        skipped: Some(skipped),
        cleaned: Some(cleaned),
        unchanged: Some(unchanged),
        failed: Some(failed),
        verified: Some(verified),
        unreadable: Some(unreadable),
        destinations,
    }
}

/// The destination paths that must survive a prune pass whatever the source
/// says about them, in one place so that the preview and the prune itself
/// cannot come to different conclusions about what is safe.
///
/// Three kinds of protection, and they exist for different reasons:
///
/// - **excluded** — the user said "don't copy this". Deleting it from the
///   destination would be the opposite of what they asked (#2).
/// - **unreadable** — the source walk could not enumerate it, so it
///   contributes nothing to the keep set. Without this guard a transient
///   permission error would read as "deleted at source" and take the whole
///   subtree with it.
/// - **patterns** — the same exclude globs, applied to destination paths
///   that were never in the source walk at all.
///
/// The first two sets are keyed by the *source's* spelling while the prune
/// pass walks the *destination's*. On a case-preserving filesystem those
/// differ the moment a folder is re-cased, and a literal comparison then
/// misses the protection — which deleted precisely what the user asked to
/// keep (#R6). Both sides are folded once, here.
pub(crate) struct ProtectedSet<'a> {
    excluded: HashSet<String>,
    unreadable: HashSet<String>,
    patterns: &'a glob::PatternSet,
    /// The source root itself could not be enumerated.
    source_root_unreadable: bool,
}

impl<'a> ProtectedSet<'a> {
    pub(crate) fn new(walked: &WalkResult, patterns: &'a glob::PatternSet) -> Self {
        Self::from_parts(&walked.excluded, &walked.unreadable, patterns)
    }

    pub(crate) fn from_parts(
        excluded: &HashSet<String>,
        unreadable: &HashSet<String>,
        patterns: &'a glob::PatternSet,
    ) -> Self {
        Self {
            excluded: excluded.iter().map(|s| fold_rel(s)).collect(),
            unreadable: unreadable.iter().map(|s| fold_rel(s)).collect(),
            patterns,
            // An empty relative path is the root's own.
            source_root_unreadable: unreadable.contains(""),
        }
    }

    /// Whether this destination path is protected — and therefore must not
    /// be deleted, nor descended into: pruning the contents of a protected
    /// directory would still effectively delete protected data.
    pub(crate) fn covers(&self, rel: &str) -> bool {
        let probe = fold_rel(rel);
        is_root_icon_marker(rel)
            || self.excluded.contains(&probe)
            || self.unreadable.contains(&probe)
            || self.patterns.matches(rel)
    }

    /// Nothing in the destination can be shown to be orphaned when the
    /// source root itself was unreadable: the keep set is empty for reasons
    /// that have nothing to do with the user deleting anything.
    pub(crate) fn source_root_unreadable(&self) -> bool {
        self.source_root_unreadable
    }
}

/// Case-fold a relative path for comparison, on the platforms whose
/// filesystems are case-insensitive.
#[cfg(windows)]
fn fold_rel(rel: &str) -> String {
    rel.to_lowercase()
}

#[cfg(not(windows))]
fn fold_rel(rel: &str) -> String {
    rel.to_string()
}

/// Walk `root` and remove any file whose relative path is not present in
/// `keep` AND not covered by `protected`.
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
async fn prune_destination(
    root: &Path,
    keep: &KeepSet,
    protected: &ProtectedSet<'_>,
    token: &CancellationToken,
    stats: &mut PhaseStats,
) -> Result<()> {
    // The entry-by-entry guard below can never match the root's own empty
    // relative path, which is what once made prune wipe whole destinations.
    if protected.source_root_unreadable() {
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
            // The destination root's icon descriptor is protected on its own
            // terms, not via `excluded`: that set is filled in by the source
            // walk, so relying on it meant the destination only kept its
            // icon when the source root happened to have one too.
            if protected.covers(&rel_str) {
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
/// we started from, and we leave it alone. That directory-level drift is
/// `recase_dirs_phase`'s job, which runs before prune — so by the time this
/// is reached, pure directory drift no longer reads as `CaseDrift` at all.
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
pub(crate) fn same_mtime(a: SystemTime, b: SystemTime) -> bool {
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
pub(crate) fn is_icon_descriptor(rel_str: &str) -> bool {
    std::path::Path::new(rel_str)
        .file_name()
        .map(|n| n.to_string_lossy().eq_ignore_ascii_case("desktop.ini"))
        .unwrap_or(false)
}

/// The relative paths the source says the destination should hold, with the
/// lookup rules the prune pass needs.
pub(crate) struct KeepSet {
    exact: HashSet<String>,
    /// Windows only: lowercased relative path -> the source's own spelling.
    #[cfg(windows)]
    by_lowercase: std::collections::HashMap<String, String>,
}

pub(crate) enum KeepStatus<'a> {
    /// Spelled the same on both sides — leave it alone.
    Exact,
    /// The source has this file but spells it differently in case only.
    /// Carries the source's spelling.
    CaseDrift(&'a str),
    /// Genuinely gone from the source.
    Absent,
}

impl KeepSet {
    pub(crate) fn new(rels: impl Iterator<Item = String>) -> Self {
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

    pub(crate) fn status(&self, rel: &str) -> KeepStatus<'_> {
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
pub(crate) struct WalkResult {
    pub(crate) files: Vec<FileEntry>,
    pub(crate) dirs: Vec<(PathBuf, String)>,
    pub(crate) total_bytes: u64,
    pub(crate) skipped: usize,
    /// Relative paths the user's exclude patterns matched.
    pub(crate) excluded: HashSet<String>,
    /// Relative paths we could not fully read: directories whose listing
    /// failed or was cut short, files we could not stat, entries whose type
    /// we could not determine. The prune pass must treat these exactly like
    /// exclusions — see `prune_destination`.
    pub(crate) unreadable: HashSet<String>,
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

pub(crate) fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) async fn walk(root: &Path, patterns: &glob::PatternSet) -> Result<WalkResult> {
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

/// `on_progress` receives signed byte deltas: positive for streamed chunks,
/// one negative correction when an attempt fails or is cancelled (the
/// worker takes its own bytes back out of the shared counter). On success,
/// returns the xxh3 of the bytes written.
async fn copy_with_retries<F: FnMut(i64)>(
    src: &Path,
    dest: &Path,
    token: &CancellationToken,
    settings: &Settings,
    on_progress: &mut F,
) -> Result<u64> {
    let mut attempts = 0;
    let max = 3;
    loop {
        attempts += 1;
        let res = copy_file(src, dest, token, settings, on_progress).await;
        match res {
            Ok(hash) => return Ok(hash),
            Err(e) => {
                if token.is_cancelled() {
                    return Err(e);
                }
                if attempts >= max {
                    // copy_file streams into a scratch file and removes it on
                    // every failure path, so there is no half-written file to
                    // clean up here (#12) — and the destination still holds
                    // the last copy that succeeded.
                    return Err(e);
                }
                let backoff = 150u64 * (1u64 << (attempts - 1));
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            }
        }
    }
}

async fn copy_file<F: FnMut(i64)>(
    src: &Path,
    dest: &Path,
    token: &CancellationToken,
    settings: &Settings,
    on_progress: &mut F,
) -> Result<u64> {
    let src_l = long_path(src);
    let dest_l = long_path(dest);
    let tmp = scratch_path(dest);
    let tmp_l = long_path(&tmp);
    let src_meta = fs::metadata(&src_l).await.context("stat source")?;
    let mut reader = fs::File::open(&src_l).await.context("open source")?;
    // Stream into the scratch file rather than over the destination: until
    // the rename at the bottom the previous backup is still whole, so an
    // unreadable source, a write error or a cancellation costs nothing that
    // was already copied. Opening the source first also means a locked file
    // fails before anything at the destination has been disturbed.
    // A scratch file left by a killed run may still carry +R.
    clear_readonly(&tmp_l);
    let mut writer = fs::File::create(&tmp_l)
        .await
        .context("create destination")?;

    let buf_size = if src_meta.len() > 4 * 1024 * 1024 {
        1024 * 1024
    } else {
        256 * 1024
    };
    let mut buf = vec![0u8; buf_size];
    let mut file_so_far: u64 = 0;
    // Hash while the bytes are already in memory — this is what lets the
    // verify phase re-read only the destination instead of both sides.
    let mut hasher = Xxh3::new();

    let streamed: Result<()> = async {
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    return Err(anyhow!(CANCELLED_MSG));
                }
                read = reader.read(&mut buf) => {
                    let n = read.context("read source")?;
                    if n == 0 { break; }
                    // Wait for the ceiling *before* writing, and wait
                    // cancellably: at a low ceiling one chunk's worth of
                    // budget is seconds, and a Stop that only lands between
                    // chunks would sit there visibly doing nothing.
                    tokio::select! {
                        biased;
                        _ = token.cancelled() => {
                            return Err(anyhow!(CANCELLED_MSG));
                        }
                        _ = ratelimit::shared().acquire(n as u64) => {}
                    }
                    writer.write_all(&buf[..n]).await.context("write destination")?;
                    hasher.update(&buf[..n]);
                    file_so_far += n as u64;
                    on_progress(n as i64);
                }
            }
        }
        writer.flush().await?;
        // Durability: actually commit to disk before returning success.
        writer.sync_all().await.context("sync destination")
    }
    .await;
    drop(writer);

    if let Err(e) = streamed {
        // Take this attempt's bytes back out of the shared counter, and
        // the partial file with them — failure and cancellation alike.
        on_progress(-(file_so_far as i64));
        let _ = fs::remove_file(&tmp_l).await;
        return Err(e);
    }

    if settings.preserve_mtime() {
        if let Ok(ft) = src_meta.modified() {
            let ft = FileTime::from_system_time(ft);
            let _ = filetime::set_file_mtime(&tmp_l, ft);
        }
    }
    // Preserve Hidden / System / ReadOnly so things like `desktop.ini`
    // (which drives custom Windows folder icons) keep their attributes.
    if let Some(attrs) = read_attrs(src) {
        apply_attrs(&tmp, attrs);
    }
    // MoveFileEx will not replace a +R file, so the bit has to come off the
    // outgoing copy. Not off the scratch file: it may legitimately carry the
    // attribute forward from the source.
    clear_readonly(&dest_l);
    fs::rename(&tmp_l, &dest_l)
        .await
        .context("commit destination")?;
    Ok(hasher.digest())
}

async fn verify_files(
    hashes: &[(String, u64)],
    backup_path: &Path,
    token: &CancellationToken,
) -> Result<()> {
    for (rel, src_hash) in hashes {
        if token.is_cancelled() {
            return Err(anyhow!(CANCELLED_MSG));
        }
        let got = hash_file(&backup_path.join(rel)).await?;
        if got != *src_hash {
            return Err(anyhow!("Hash mismatch for {}", rel));
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

    /// Drive a single destination the way `execute_all` does, for the tests
    /// that care about what happens *inside* one destination rather than
    /// across several. `execute_all` turns a destination's failure into an
    /// outcome and keeps going — which is the point of it — so it is the
    /// wrong entry point for asserting that a copy failed.
    async fn run_one_destination<R: Runtime>(
        app: &AppHandle<R>,
        backup_id: &str,
        task: &Task,
        dest: &Path,
        settings: &Settings,
        token: &CancellationToken,
    ) -> Result<DestinationOutcome> {
        let patterns = glob::PatternSet::from_input(&settings.exclude_patterns);
        let mut walked = walk(Path::new(&task.source), &patterns).await?;
        let protected = ProtectedSet::new(&walked, &patterns);
        execute_one(
            app,
            backup_id,
            task,
            dest,
            0,
            1,
            &mut walked,
            &protected,
            settings,
            token,
        )
        .await
    }

    /// Cancelling trips the token but must leave the task registered. The
    /// run is still draining its in-flight copies; freeing the slot here let
    /// a second run start against the same destination, and the two deleted
    /// each other's files while reporting success (#R3).
    #[test]
    fn cancelling_keeps_the_slot_taken_while_the_run_drains() {
        let state = BackupState::default();
        let (_run, token) = state.try_register("t").expect("an idle task registers");

        state.cancel("t");

        assert!(token.is_cancelled(), "cancel must trip the token");
        assert!(
            state.is_active("t"),
            "the slot must stay taken until the run itself unregisters"
        );
        assert!(
            state.try_register("t").is_none(),
            "a second run must still be refused while the first drains"
        );
    }

    /// A run that finishes after a newer one has started must remove only
    /// its own registration — evicting the newcomer left it uncancellable
    /// and the slot open for a third run (#R3).
    #[test]
    fn a_late_finisher_does_not_evict_a_newer_run() {
        let state = BackupState::default();
        let (run_a, _token_a) = state.try_register("t").unwrap();
        state.unregister("t", run_a);

        let (_run_b, token_b) = state.try_register("t").expect("slot freed by run A");
        // Run A's cleanup arrives late, after B has taken the slot.
        state.unregister("t", run_a);

        assert!(state.is_active("t"), "run B's registration was evicted");
        state.cancel("t");
        assert!(token_b.is_cancelled(), "run B must still be cancellable");
    }

    /// End-to-end pipeline run against the mock Tauri runtime: copy a small
    /// tree, prune an orphan, and report honest counts. This is the harness
    /// the parallel-copy and recase regression tests build on.
    #[tokio::test]
    async fn execute_mirrors_source_into_destination_end_to_end() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("a.txt"), b"alpha").unwrap();
        std::fs::write(source.join("sub").join("b.txt"), b"beta").unwrap();
        std::fs::write(dest.join("stale.txt"), b"orphan").unwrap();

        let app = tauri::test::mock_app();
        let task = Task {
            id: "e2e".into(),
            name: "e2e".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let token = CancellationToken::new();
        let payload = execute_all(
            app.handle(),
            "backup-e2e",
            &task,
            &Settings::default(),
            &token,
        )
        .await
        .expect("pipeline should succeed");

        assert!(payload.success);
        assert!(!payload.cancelled);
        assert_eq!(payload.total_files, Some(2));
        assert_eq!(payload.failed, Some(0));
        assert_eq!(payload.cleaned, Some(1), "orphan should be pruned");
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(
            std::fs::read(dest.join("sub").join("b.txt")).unwrap(),
            b"beta"
        );
        assert!(!dest.join("stale.txt").exists());
    }

    /// Test fixture: a source tree with two files, and `count` empty
    /// destination folders beside it.
    fn tree_with_destinations(root: &Path, count: usize) -> (PathBuf, Vec<PathBuf>) {
        let source = root.join("source");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::write(source.join("a.txt"), b"alpha").unwrap();
        std::fs::write(source.join("sub").join("b.txt"), b"beta").unwrap();
        let dests = (0..count)
            .map(|i| {
                let d = root.join(format!("dest{i}"));
                std::fs::create_dir_all(&d).unwrap();
                d
            })
            .collect();
        (source, dests)
    }

    fn task_with(id: &str, source: &Path, dests: &[PathBuf]) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(
                dests
                    .iter()
                    .map(|d| d.to_string_lossy().to_string())
                    .collect(),
            ),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        }
    }

    /// The whole point of the feature: one source, several destinations,
    /// each of them a complete mirror.
    #[tokio::test]
    async fn every_destination_receives_the_whole_tree() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 3);
        let task = task_with("multi", &source, &dests);

        let payload = execute_all(
            tauri::test::mock_app().handle(),
            "backup-multi",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .expect("the run should succeed");

        assert!(payload.success);
        assert!(!payload.partial);
        assert_eq!(payload.destinations.len(), 3);
        for d in &payload.destinations {
            assert_eq!(d.status, DestinationStatus::Success);
        }
        for dest in &dests {
            assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"alpha");
            assert_eq!(std::fs::read(dest.join("sub/b.txt")).unwrap(), b"beta");
        }
        // The counts are sums: three copies of a two-file tree really did
        // move six files, and a row claiming two would describe a third of
        // the work.
        assert_eq!(payload.total_files, Some(6));
        assert_eq!(payload.path.as_deref(), Some(dests[0].to_string_lossy().as_ref()));
    }

    /// An unplugged drive among several must not cost the others their
    /// backup — and must not be reported as a plain failure either.
    #[tokio::test]
    async fn an_absent_destination_leaves_the_others_backed_up() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let absent = root.path().join("unplugged-drive");
        let mut task = task_with("partial", &source, &dests);
        task.destinations
            .as_mut()
            .unwrap()
            .push(absent.to_string_lossy().to_string());

        let payload = execute_all(
            tauri::test::mock_app().handle(),
            "backup-partial",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .expect("the reachable destination should still run");

        assert!(!payload.success, "not every destination was written");
        assert!(payload.partial, "one destination was");
        assert_eq!(payload.destinations[0].status, DestinationStatus::Success);
        assert_eq!(
            payload.destinations[1].status,
            DestinationStatus::Unreachable
        );
        assert_eq!(std::fs::read(dests[0].join("a.txt")).unwrap(), b"alpha");
        assert!(!absent.exists(), "an absent destination is never created");
    }

    /// A destination nested inside another is not in its host's keep set,
    /// so the host's prune pass would delete the whole thing — one backup
    /// erasing another. The run must be refused before anything is written,
    /// including into the destination that looks perfectly healthy.
    #[tokio::test]
    async fn nested_destinations_are_refused_before_anything_is_written() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let inner = dests[0].join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let mut task = task_with("nested", &source, &dests);
        task.destinations
            .as_mut()
            .unwrap()
            .push(inner.to_string_lossy().to_string());

        let result = execute_all(
            tauri::test::mock_app().handle(),
            "backup-nested",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await;

        assert!(result.is_err(), "overlapping destinations must be refused");
        assert!(
            !dests[0].join("a.txt").exists(),
            "nothing may be written when the configuration is unsafe"
        );
    }

    /// A tasks.json written before 1.7.2 has a single `destination` string
    /// and no array. The scheduler deserialises the same struct and can tick
    /// before the frontend has migrated the file, so this shape has to keep
    /// working on its own.
    #[tokio::test]
    async fn a_pre_1_7_2_task_backs_up_to_its_single_destination() {
        let root = tempfile::tempdir().unwrap();
        let (source, dests) = tree_with_destinations(root.path(), 1);
        let task = Task {
            id: "legacy".into(),
            name: "legacy".into(),
            source: source.to_string_lossy().to_string(),
            destination: Some(dests[0].to_string_lossy().to_string()),
            destinations: None,
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };

        let payload = execute_all(
            tauri::test::mock_app().handle(),
            "backup-legacy",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await
        .expect("a legacy task still runs");

        assert!(payload.success);
        assert_eq!(payload.destinations.len(), 1);
        assert_eq!(std::fs::read(dests[0].join("a.txt")).unwrap(), b"alpha");
    }

    #[tokio::test]
    async fn a_task_with_no_destination_at_all_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let (source, _) = tree_with_destinations(root.path(), 0);
        let task = task_with("empty", &source, &[]);

        let result = execute_all(
            tauri::test::mock_app().handle(),
            "backup-empty",
            &task,
            &Settings::default(),
            &CancellationToken::new(),
        )
        .await;

        assert!(result.is_err());
    }

    /// The conversion from what the user typed to what the bucket counts.
    /// A factor-of-1000 slip here is invisible in the UI and turns a 50 MB/s
    /// ceiling into a 50 KB/s one.
    #[test]
    fn max_speed_reads_as_mebibytes_per_second() {
        let with = |mbps| Settings {
            max_speed_mbps: mbps,
            ..Default::default()
        }
        .max_speed_bytes();

        assert_eq!(with(Some(50.0)), 50 * 1024 * 1024);
        assert_eq!(with(Some(0.5)), 512 * 1024);
        // Every way of saying "no ceiling", including the ones a hand-edited
        // settings.json can produce.
        assert_eq!(with(None), 0);
        assert_eq!(with(Some(0.0)), 0);
        assert_eq!(with(Some(-5.0)), 0);
        assert_eq!(with(Some(f64::NAN)), 0);
        // Absurd but harmless: the cast saturates rather than wrapping to a
        // tiny ceiling that would stall every copy.
        assert!(with(Some(f64::INFINITY)) > 0);
    }

    #[test]
    fn task_destinations_normalises_both_shapes() {
        let base = Task {
            id: "t".into(),
            name: "t".into(),
            source: "C:/src".into(),
            destination: None,
            destinations: None,
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };

        // The plural field wins over a leftover singular one.
        let both = Task {
            destination: Some("D:/old".into()),
            destinations: Some(vec!["E:/new".into()]),
            ..base.clone()
        };
        assert_eq!(both.destinations(), vec!["E:/new".to_string()]);

        // Blanks and exact repeats go; order is the user's.
        let messy = Task {
            destinations: Some(vec![
                "E:/b".into(),
                "  ".into(),
                " D:/a ".into(),
                "E:/b".into(),
            ]),
            ..base.clone()
        };
        assert_eq!(
            messy.destinations(),
            vec!["E:/b".to_string(), "D:/a".to_string()]
        );

        // An empty array is not a destination list — fall back to the
        // legacy field rather than reporting none.
        let empty_array = Task {
            destination: Some("D:/only".into()),
            destinations: Some(vec![]),
            ..base.clone()
        };
        assert_eq!(empty_array.destinations(), vec!["D:/only".to_string()]);
        assert!(base.destinations().is_empty());
    }

    /// A folder the user re-cased at source keeps the destination's stale
    /// spelling forever: 1.5 stopped prune from *deleting* files through the
    /// drifted directory, and 1.6 actually re-spells the directory entry.
    #[cfg(windows)]
    #[tokio::test]
    async fn execute_respells_a_recased_directory_at_the_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(source.join("Docs")).unwrap();
        std::fs::write(source.join("Docs").join("readme.md"), b"hello").unwrap();
        std::fs::create_dir_all(dest.join("docs")).unwrap();
        std::fs::write(dest.join("docs").join("readme.md"), b"hello").unwrap();

        let app = tauri::test::mock_app();
        let task = Task {
            id: "recase".into(),
            name: "recase".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let token = CancellationToken::new();
        let payload = execute_all(
            app.handle(),
            "backup-recase",
            &task,
            &Settings::default(),
            &token,
        )
        .await
        .unwrap();

        assert!(payload.success);
        assert_eq!(
            payload.cleaned,
            Some(0),
            "a case-only rename must not delete anything"
        );
        let names: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            names.contains(&"Docs".to_string()),
            "destination folder kept its stale casing: {:?}",
            names
        );
        assert_eq!(
            std::fs::read(dest.join("Docs").join("readme.md")).unwrap(),
            b"hello"
        );
    }

    /// Multi-level drift: every re-cased component of the path gets its
    /// source spelling back, not just the deepest one.
    #[cfg(windows)]
    #[tokio::test]
    async fn execute_respells_nested_recased_directories() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(source.join("Alpha").join("Beta")).unwrap();
        std::fs::write(source.join("Alpha").join("Beta").join("c.txt"), b"x").unwrap();
        std::fs::create_dir_all(dest.join("alpha").join("beta")).unwrap();
        std::fs::write(dest.join("alpha").join("beta").join("c.txt"), b"x").unwrap();

        let app = tauri::test::mock_app();
        let task = Task {
            id: "recase-nested".into(),
            name: "recase-nested".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let token = CancellationToken::new();
        let payload = execute_all(
            app.handle(),
            "backup-recase-nested",
            &task,
            &Settings::default(),
            &token,
        )
        .await
        .unwrap();
        assert!(payload.success);

        let top: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(top.contains(&"Alpha".to_string()), "top level: {:?}", top);
        let inner: Vec<String> = std::fs::read_dir(dest.join("Alpha"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(inner.contains(&"Beta".to_string()), "inner level: {:?}", inner);
        assert!(dest.join("Alpha").join("Beta").join("c.txt").exists());
    }

    /// The parallel copy path must produce byte-identical results and the
    /// same counts as the sequential one — `parallelCopies: 1` is the
    /// escape hatch and has to stay a faithful baseline.
    #[tokio::test]
    async fn parallel_copies_match_sequential_results() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        for i in 0..40u32 {
            std::fs::write(
                source.join(format!("f{i}.bin")),
                vec![i as u8; 1000 + i as usize],
            )
            .unwrap();
            std::fs::write(
                source.join("nested").join(format!("n{i}.bin")),
                vec![i as u8; 10 + i as usize],
            )
            .unwrap();
        }

        let app = tauri::test::mock_app();
        let mut payloads = Vec::new();
        for (name, n) in [("seq", 1u32), ("par", 4u32)] {
            let dest = root.path().join(format!("dest-{name}"));
            std::fs::create_dir_all(&dest).unwrap();
            let task = Task {
                id: name.into(),
                name: name.into(),
                source: source.to_string_lossy().to_string(),
                destination: None,
                destinations: Some(vec![dest.to_string_lossy().to_string()]),
                schedule: None,
                schedule_days: None,
                schedule_time: None,
                last_backup: None,
            };
            let settings = Settings {
                parallel_copies: Some(n),
                ..Default::default()
            };
            let token = CancellationToken::new();
            let payload = execute_all(app.handle(), name, &task, &settings, &token)
                .await
                .unwrap();
            assert!(payload.success);
            assert_eq!(std::fs::read(dest.join("f7.bin")).unwrap(), vec![7u8; 1007]);
            assert_eq!(
                std::fs::read(dest.join("nested").join("n39.bin")).unwrap(),
                vec![39u8; 49]
            );
            payloads.push(payload);
        }
        let (seq, par) = (&payloads[0], &payloads[1]);
        assert_eq!(seq.total_files, par.total_files);
        assert_eq!(seq.total_bytes, par.total_bytes);
        assert_eq!(seq.failed, par.failed);
        assert_eq!(seq.unchanged, par.unchanged);
        assert_eq!(par.total_files, Some(80));
        assert_eq!(par.failed, Some(0));
    }

    /// Every in-flight parallel copy must take its partial file back out on
    /// cancellation, exactly like the sequential loop always has.
    #[tokio::test]
    async fn a_cancelled_parallel_copy_leaves_no_partial_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        for i in 0..10u8 {
            std::fs::write(source.join(format!("f{i}.bin")), vec![i; 1024 * 1024]).unwrap();
        }

        let app = tauri::test::mock_app();
        let task = Task {
            id: "cancel-par".into(),
            name: "cancel-par".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let settings = Settings {
            parallel_copies: Some(4),
            ..Default::default()
        };
        let token = CancellationToken::new();
        token.cancel();

        let result =
            run_one_destination(app.handle(), "cancel-par", &task, &dest, &settings, &token)
                .await;
        assert!(result.is_err(), "a cancelled run must not report success");
        let leftover: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            leftover.is_empty(),
            "cancelled run left partial files: {:?}",
            leftover
        );
    }

    /// A source we cannot open must not cost us the copy we already hold.
    /// The destination used to be deleted at the top of the retry loop,
    /// before anything had tried to open the source, so a single locked file
    /// destroyed its own backup and nothing replaced it (#R1).
    #[cfg(windows)]
    #[tokio::test]
    async fn an_unreadable_source_leaves_the_previous_backup_intact() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Last night's run already put a good copy in the destination.
        const PREVIOUS: &[u8] = b"the previous good backup";
        std::fs::write(source.join("vm.bin"), b"newer contents").unwrap();
        std::fs::write(dest.join("vm.bin"), PREVIOUS).unwrap();
        // Something else keeps a plain readable file alongside it, so the run
        // has real work to do and doesn't bail for unrelated reasons.
        std::fs::write(source.join("notes.txt"), b"readable").unwrap();

        // Tonight the source is held open with no sharing — a running VM
        // holding its own disk image.
        let _locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(source.join("vm.bin"))
            .unwrap();

        let app = tauri::test::mock_app();
        let task = Task {
            id: "locked-src".into(),
            name: "locked-src".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let settings = Settings {
            continue_on_error: Some(true),
            ..Default::default()
        };
        let token = CancellationToken::new();

        let _ =
            run_one_destination(app.handle(), "locked-src", &task, &dest, &settings, &token).await;

        assert_eq!(
            std::fs::read(dest.join("vm.bin")).unwrap(),
            PREVIOUS,
            "an unreadable source destroyed the backup it could not replace"
        );
        // And no scratch file is left sitting in the destination.
        let leftovers: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".driveby-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "scratch files left behind: {:?}", leftovers);
    }

    /// continueOnError=false stops the run on the first real failure — and
    /// that abort must never be mistaken for a user cancellation, which is
    /// what run_backup reads off the token afterwards.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_failing_file_aborts_the_run_without_reading_as_cancelled() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        for i in 0..8u8 {
            std::fs::write(source.join(format!("ok{i}.bin")), vec![i; 512]).unwrap();
        }
        std::fs::write(source.join("locked.bin"), b"unreadable").unwrap();
        let _locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(source.join("locked.bin"))
            .unwrap();

        let app = tauri::test::mock_app();
        let task = Task {
            id: "abort".into(),
            name: "abort".into(),
            source: source.to_string_lossy().to_string(),
            destination: None,
            destinations: Some(vec![dest.to_string_lossy().to_string()]),
            schedule: None,
            schedule_days: None,
            schedule_time: None,
            last_backup: None,
        };
        let settings = Settings {
            continue_on_error: Some(false),
            parallel_copies: Some(4),
            ..Default::default()
        };
        let token = CancellationToken::new();

        let result =
            run_one_destination(app.handle(), "abort", &task, &dest, &settings, &token).await;
        assert!(result.is_err(), "the run must abort on the locked file");
        assert!(
            !token.is_cancelled(),
            "an abort must not read as user cancellation"
        );
    }

    /// verify now checks captured copy-time hashes against a re-read of the
    /// destination only — the redesign that stops verify doubling the I/O.
    #[tokio::test]
    async fn verify_files_flags_a_corrupted_destination() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.bin"), b"corrupted-content").unwrap();
        let token = CancellationToken::new();

        let mut expected = Xxh3::new();
        expected.update(b"expected-content");
        let bad = vec![("file.bin".to_string(), expected.digest())];
        assert!(
            verify_files(&bad, root.path(), &token).await.is_err(),
            "a hash mismatch must fail verification"
        );

        let mut actual = Xxh3::new();
        actual.update(b"corrupted-content");
        let good = vec![("file.bin".to_string(), actual.digest())];
        assert!(verify_files(&good, root.path(), &token).await.is_ok());
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
            &ProtectedSet::from_parts(&excluded, &unreadable, &glob::PatternSet::new(&[])),
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
            &ProtectedSet::from_parts(&empty, &empty, &glob::PatternSet::new(&[])),
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

    /// The destination root's own `desktop.ini` is what gives the backup
    /// drive its icon in Explorer, and prune_destination's contract says it
    /// is never removed. It only ever was, though, when the *source* root
    /// happened to carry one too: the guard read from the `excluded` set,
    /// which the source walk fills in. Back up a source whose root has no
    /// desktop.ini and the destination lost its icon on the first run.
    ///
    /// A *nested* desktop.ini is a real sub-folder's icon and still tracks
    /// the source like any other file.
    #[tokio::test]
    async fn prune_never_removes_the_destination_roots_icon_descriptor() {
        let root = scratch("prune-root-icon");
        std::fs::write(root.join("desktop.ini"), b"[.ShellClassInfo]").unwrap();
        std::fs::write(root.join("orphan.txt"), b"stale").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/desktop.ini"), b"nested").unwrap();

        let empty: HashSet<String> = HashSet::new();
        let token = CancellationToken::new();
        let mut stats = PhaseStats::default();
        prune_destination(
            &root,
            &KeepSet::new(std::iter::empty()),
            &ProtectedSet::from_parts(&empty, &empty, &glob::PatternSet::new(&[])),
            &token,
            &mut stats,
        )
        .await
        .unwrap();

        assert!(
            root.join("desktop.ini").exists(),
            "prune removed the destination root's icon descriptor"
        );
        assert!(!root.join("orphan.txt").exists());
        assert!(!root.join("sub/desktop.ini").exists());
        assert_eq!(stats.deleted, 2);
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
            &ProtectedSet::from_parts(&empty, &empty, &glob::PatternSet::new(&[])),
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
            &ProtectedSet::from_parts(&empty, &empty, &glob::PatternSet::new(&[])),
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
        copy_with_retries(&src, &dest, &token, &settings, &mut |_| {})
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
            &ProtectedSet::from_parts(&empty, &unreadable, &glob::PatternSet::new(&[])),
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
