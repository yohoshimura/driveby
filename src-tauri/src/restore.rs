use crate::fsutil::{apply_attrs, clear_readonly, long_path, read_attrs, reject_overlap};
use anyhow::{anyhow, Context, Result};
use filetime::FileTime;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

/// Cooperative-cancellation sentinel, same convention as the backup
/// pipeline's: the inner loop short-circuits with this message and the
/// outer `run_restore()` consults the token — never this string — to decide
/// whether the run was cancelled.
const CANCELLED_MSG: &str = "restore cancelled";

/// One restore at a time, process-wide. Restores are not task-keyed the way
/// backups are — the UI exposes a single restore flow — so a single slot is
/// the honest shape, not a map.
#[derive(Default)]
pub struct RestoreState(Mutex<Option<CancellationToken>>);

impl RestoreState {
    /// Atomic begin-if-idle. Returns None when a restore is already
    /// running — the caller must not start a second one.
    fn try_begin(&self) -> Option<CancellationToken> {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() {
            return None;
        }
        let token = CancellationToken::new();
        *slot = Some(token.clone());
        Some(token)
    }
    pub fn cancel(&self) {
        let slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(token) = slot.as_ref() {
            token.cancel();
        }
    }
    fn finish(&self) {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *slot = None;
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RestorePayload {
    pub backup_path: String,
    pub destination: String,
    pub success: bool,
    pub cancelled: bool,
    pub copied_files: u64,
    pub total_files: u64,
    pub total_bytes: u64,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RestoreProgress {
    copied_files: u64,
    total_files: u64,
    copied_bytes: u64,
    total_bytes: u64,
    progress: u32,
}

/// The command-facing entry point: registers the run in `state` (refusing a
/// concurrent one), threads the cancellation token through the pipeline,
/// and owns the cancelled-vs-failed mapping — mirroring `backup::run_backup`.
pub async fn run_restore<R: Runtime>(
    app: &AppHandle<R>,
    state: &RestoreState,
    backup_path: PathBuf,
    destination: PathBuf,
) -> Result<RestorePayload> {
    let token = match state.try_begin() {
        Some(t) => t,
        None => return Err(anyhow!("A restore is already running")),
    };

    let result = restore(app, &token, backup_path.clone(), destination.clone()).await;

    // The token, not the error string, is the source of truth for
    // cancellation — same rationale as run_backup.
    let cancelled = token.is_cancelled();
    state.finish();
    conclude(result, cancelled, &backup_path, &destination)
}

/// Fold cancellation state into the payload: a cancelled run reports
/// `cancelled: true` with no error string, whatever shape the pipeline's
/// result took; anything else passes through untouched.
fn conclude(
    result: Result<RestorePayload>,
    cancelled: bool,
    backup_path: &Path,
    destination: &Path,
) -> Result<RestorePayload> {
    match result {
        Ok(mut p) => {
            if cancelled {
                p.success = false;
                p.cancelled = true;
                p.error = None;
            }
            Ok(p)
        }
        Err(_) if cancelled => Ok(RestorePayload {
            backup_path: backup_path.to_string_lossy().to_string(),
            destination: destination.to_string_lossy().to_string(),
            success: false,
            cancelled: true,
            copied_files: 0,
            total_files: 0,
            total_bytes: 0,
            duration_ms: 0,
            error: None,
        }),
        Err(e) => Err(e),
    }
}

/// Generic over the Tauri runtime purely so tests can drive the real
/// pipeline against `tauri::test::mock_app()`. Production passes an
/// `AppHandle` (i.e. `AppHandle<Wry>`) and infers `R` from it, so no call
/// site changes.
async fn restore<R: Runtime>(
    app: &AppHandle<R>,
    token: &CancellationToken,
    backup_path: PathBuf,
    destination: PathBuf,
) -> Result<RestorePayload> {
    if !backup_path.is_absolute() || !destination.is_absolute() {
        return Err(anyhow!("Paths must be absolute"));
    }
    let src_meta = fs::metadata(long_path(&backup_path))
        .await
        .map_err(|_| anyhow!("Backup folder not found"))?;
    if !src_meta.is_dir() {
        return Err(anyhow!("Backup path is not a directory"));
    }
    let dest_meta = fs::metadata(long_path(&destination))
        .await
        .map_err(|_| anyhow!("Destination not found"))?;
    if !dest_meta.is_dir() {
        return Err(anyhow!("Destination is not a directory"));
    }
    // Without this, restoring a backup *onto itself* destroys it: for every
    // file `dst == src`, and `File::create(dst)` truncates the very file the
    // already-open `File::open(src)` handle is about to read — so each file
    // is read as 0 bytes, written as 0 bytes, and the run still reports
    // success. The backup pipeline has always had this guard; restore did
    // not, even though it is reachable from the UI in two clicks (History →
    // Restore → pick the backup folder as the destination).
    reject_overlap(&backup_path, &destination)?;

    let files = walk(&backup_path).await?;
    let total_files = files.len() as u64;
    let total_bytes: u64 = files.iter().map(|(_, _, size)| *size).sum();

    let started = Instant::now();
    let mut copied_bytes: u64 = 0;
    let mut copied_files: u64 = 0;
    // Throttle to one event per 100ms, like the backup pipeline — the old
    // per-file emit flooded the webview on trees of small files.
    let mut last_emit = Instant::now();

    for (rel, src, size) in &files {
        let dst = long_path(&destination.join(rel));
        if let Err(e) = restore_one(src, &dst, token).await {
            return Ok(RestorePayload {
                backup_path: backup_path.to_string_lossy().to_string(),
                destination: destination.to_string_lossy().to_string(),
                success: false,
                cancelled: false,
                copied_files,
                total_files,
                total_bytes,
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some(format!("{}: {}", rel, e)),
            });
        }
        copied_files += 1;
        copied_bytes += *size;
        if last_emit.elapsed().as_millis() >= 100 {
            last_emit = Instant::now();
            emit_progress(app, copied_files, total_files, copied_bytes, total_bytes);
        }
    }

    // Unconditional final emit so the UI lands on 100% even when the
    // throttle swallowed the last in-loop event.
    emit_progress(app, copied_files, total_files, copied_bytes, total_bytes);

    Ok(RestorePayload {
        backup_path: backup_path.to_string_lossy().to_string(),
        destination: destination.to_string_lossy().to_string(),
        success: true,
        cancelled: false,
        copied_files,
        total_files,
        total_bytes,
        duration_ms: started.elapsed().as_millis() as u64,
        error: None,
    })
}

fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    copied_files: u64,
    total_files: u64,
    copied_bytes: u64,
    total_bytes: u64,
) {
    let progress = if total_bytes > 0 {
        ((copied_bytes as f64 / total_bytes as f64) * 100.0).min(100.0) as u32
    } else {
        0
    };
    let _ = app.emit(
        "restore-progress",
        RestoreProgress {
            copied_files,
            total_files,
            copied_bytes,
            total_bytes,
            progress,
        },
    );
}

// Returns (relative path, absolute source path, size). Surfaces I/O errors
// instead of silently truncating the iteration like the v1.1.0 walker did
// (#6) — a missed file in a "successful" restore is a quiet data-loss bug.
async fn walk(root: &Path) -> Result<Vec<(String, PathBuf, u64)>> {
    let mut out = Vec::new();
    // Extended-length form throughout, so a backup of a deep tree — which
    // the backup pipeline handles fine — is also restorable.
    let root = long_path(root);
    let root = root.as_path();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("read_dir {}", dir.display()))?;
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => return Err(anyhow!("read_dir entry in {}: {}", dir.display(), e)),
            };
            let path = entry.path();
            let ft = entry
                .file_type()
                .await
                .with_context(|| format!("file_type {}", path.display()))?;
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let meta = fs::metadata(&path)
                    .await
                    .with_context(|| format!("metadata {}", path.display()))?;
                let rel = match path.strip_prefix(root) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.push((rel_str, path.clone(), meta.len()));
            }
        }
    }
    Ok(out)
}

/// Restore one file of the backup into the destination tree.
///
/// Cleanup on failure is deliberately narrow. The restore loop used to drop
/// `dst` on *any* error from `copy`, but `copy` can fail before it ever
/// opens the destination — `stat source` and `open source` both come first.
/// An unreadable file in the backup therefore deleted the good file the
/// user already had at the destination, which is the exact opposite of what
/// a restore is for. Only `copy` knows whether it got as far as creating
/// the file, so only `copy` takes it back out again.
async fn restore_one(src: &Path, dst: &Path, token: &CancellationToken) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).await?;
    }
    copy(src, dst, token).await
}

async fn copy(src: &Path, dst: &Path, token: &CancellationToken) -> Result<()> {
    let src = long_path(src);
    let dst = long_path(dst);
    let (src, dst) = (src.as_path(), dst.as_path());
    let src_meta = fs::metadata(src).await.context("stat source")?;
    let mut r = fs::File::open(src).await.context("open source")?;
    // A read-only file already sitting in the chosen destination would make
    // File::create fail outright on Windows and abort the whole restore.
    clear_readonly(dst);
    let mut w = fs::File::create(dst).await.context("create destination")?;
    // Past this line the destination holds our bytes rather than the user's,
    // so a failure has to take the half-written file back out — otherwise a
    // re-run can collide on size and quietly accept the truncated copy.
    let streamed = async {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    // The Err return routes through the same cleanup below
                    // that failed copies use, so no partial file survives.
                    return Err(anyhow!(CANCELLED_MSG));
                }
                read = r.read(&mut buf) => {
                    let n = read.context("read source")?;
                    if n == 0 {
                        break;
                    }
                    w.write_all(&buf[..n]).await.context("write destination")?;
                }
            }
        }
        w.flush().await?;
        // Durability: don't claim "restored" before the bytes hit disk.
        w.sync_all().await.context("sync destination")
    }
    .await;
    drop(w);
    if let Err(e) = streamed {
        clear_readonly(dst);
        let _ = fs::remove_file(dst).await;
        return Err(e);
    }
    // Round-trip mtime so a follow-up sync doesn't re-copy everything (#6).
    if let Ok(t) = src_meta.modified() {
        let _ = filetime::set_file_mtime(dst, FileTime::from_system_time(t));
    }
    // Mirror Hidden/System/ReadOnly the way the backup pipeline does — a
    // restored `desktop.ini` without its attributes leaves the folder
    // rendering with a default icon, which is precisely the thing the
    // backup side goes to some length to preserve.
    if let Some(attrs) = read_attrs(src) {
        apply_attrs(dst, attrs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The single-slot registry: one restore at a time, process-wide. A
    /// second begin while one is running must be refused, and the slot must
    /// be reusable once the first finishes.
    #[test]
    fn a_second_restore_is_refused_while_one_is_running() {
        let state = RestoreState::default();
        let first = state.try_begin().expect("idle state accepts a restore");
        assert!(
            state.try_begin().is_none(),
            "a concurrent restore must be refused"
        );
        state.finish();
        assert!(
            state.try_begin().is_some(),
            "the slot must be reusable after finish()"
        );
        drop(first);
    }

    /// Cancellation must take the in-flight destination file back out, the
    /// same way a failed copy does. A pre-cancelled token makes the timing
    /// deterministic: copy() creates the destination, then the biased select
    /// sees the cancellation before the first chunk lands.
    #[tokio::test]
    async fn a_cancelled_restore_takes_its_partial_file_back_out() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(backup.join("file.bin"), vec![7u8; 1024 * 1024]).unwrap();

        let app = tauri::test::mock_app();
        let token = CancellationToken::new();
        token.cancel();

        let result = restore(app.handle(), &token, backup, dest.clone()).await;

        assert!(
            !dest.join("file.bin").exists(),
            "a cancelled restore left a partial file behind"
        );
        // The loop reports the cancelled copy like any other per-file
        // failure; the cancelled/success mapping is conclude()'s job.
        let payload = result.expect("cancellation is not a hard error");
        assert!(!payload.success);
    }

    /// conclude() owns the cancelled-vs-failed distinction, mirroring the
    /// backup pipeline: a cancelled run reports cancelled=true with no error
    /// string, never "failed".
    #[test]
    fn conclude_reports_a_cancelled_run_as_cancelled_not_failed() {
        let payload = RestorePayload {
            backup_path: "b".into(),
            destination: "d".into(),
            success: false,
            cancelled: false,
            copied_files: 3,
            total_files: 10,
            total_bytes: 100,
            duration_ms: 5,
            error: Some("file.bin: restore cancelled".into()),
        };
        let out = conclude(Ok(payload), true, Path::new("b"), Path::new("d")).unwrap();
        assert!(out.cancelled);
        assert!(!out.success);
        assert!(out.error.is_none(), "cancellation is not an error");
        assert_eq!(out.copied_files, 3, "progress so far should be kept");

        // A hard pre-flight error during a cancelled run still maps to a
        // cancelled payload rather than surfacing as Err.
        let out = conclude(
            Err(anyhow!("boom")),
            true,
            Path::new("b"),
            Path::new("d"),
        )
        .unwrap();
        assert!(out.cancelled);

        // And without cancellation, errors stay errors.
        assert!(conclude(Err(anyhow!("boom")), false, Path::new("b"), Path::new("d")).is_err());
    }

    /// Unlike the backup pipeline, restore has no delete-then-create dance:
    /// it goes straight to `File::create`, which is a hard PermissionDenied
    /// against an existing read-only file. Any read-only file already living
    /// in the chosen destination therefore aborted the whole restore.
    #[cfg(windows)]
    #[tokio::test]
    async fn copy_overwrites_readonly_destination() {
        let root = std::env::temp_dir().join("driveby-restore-test-readonly");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.txt");
        let dst = root.join("dst.txt");
        std::fs::write(&src, b"restored").unwrap();
        std::fs::write(&dst, b"read-only leftover").unwrap();
        crate::fsutil::apply_attrs(&dst, 0x1);

        copy(&src, &dst, &CancellationToken::new())
            .await
            .expect("a read-only file in the destination must not abort the restore");
        assert_eq!(std::fs::read(&dst).unwrap(), b"restored");

        crate::fsutil::clear_readonly(&dst);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The restore loop used to drop the destination file on *any* copy
    /// error, including the ones raised before `File::create` is reached.
    /// A backup file that cannot be read therefore destroyed the perfectly
    /// good file the user already had at the destination — the opposite of
    /// what a restore is for.
    #[tokio::test]
    async fn a_copy_that_never_opened_the_destination_leaves_it_alone() {
        let root = std::env::temp_dir().join("driveby-restore-test-unreadable-src");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("missing-in-backup.txt");
        let dst = root.join("report.docx");
        std::fs::write(&dst, b"the user's existing file").unwrap();

        assert!(
            restore_one(&src, &dst, &CancellationToken::new())
                .await
                .is_err(),
            "source does not exist"
        );
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"the user's existing file",
            "restore deleted a destination file it never wrote to"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The same guarantee, driven through the real entry point rather than a
    /// single helper: `restore()` walks the backup, hits a file it cannot
    /// open, and must leave the user's copy at the destination untouched.
    ///
    /// The unreadable backup file is simulated by holding an exclusive
    /// handle on it (`share_mode(0)`), which is what a file open in another
    /// program looks like — `File::open` then fails with a sharing
    /// violation, before `copy` ever reaches `File::create`.
    #[cfg(windows)]
    #[tokio::test]
    async fn restore_leaves_the_destination_alone_when_the_backup_is_unreadable() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = std::env::temp_dir().join("driveby-restore-test-locked-backup");
        let _ = std::fs::remove_dir_all(&root);
        let backup = root.join("backup");
        let dest = root.join("dest");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(backup.join("report.docx"), b"backup copy").unwrap();
        std::fs::write(dest.join("report.docx"), b"the user's existing file").unwrap();

        // Deny all sharing, so the restore cannot open the backup's copy.
        let _locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(backup.join("report.docx"))
            .unwrap();

        let app = tauri::test::mock_app();
        let payload = restore(
            app.handle(),
            &CancellationToken::new(),
            backup.clone(),
            dest.clone(),
        )
        .await
        .expect("restore reports failure in its payload, not as an Err");

        assert!(!payload.success, "the run should be reported as failed");
        assert_eq!(
            std::fs::read(dest.join("report.docx")).unwrap(),
            b"the user's existing file",
            "restore destroyed the file it was called to protect"
        );

        drop(_locked);
        let _ = std::fs::remove_dir_all(&root);
    }
}
