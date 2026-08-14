use crate::fsutil::{apply_attrs, clear_readonly, long_path, read_attrs, reject_overlap};
use anyhow::{anyhow, Context, Result};
use filetime::FileTime;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RestorePayload {
    pub backup_path: String,
    pub destination: String,
    pub success: bool,
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

pub async fn restore(
    app: &AppHandle,
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

    for (rel, src, size) in &files {
        let dst = long_path(&destination.join(rel));
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).await?;
        }
        if let Err(e) = copy(src, &dst).await {
            // Drop the half-written file so a re-run of restore doesn't
            // collide on size and quietly accept it.
            clear_readonly(&dst);
            let _ = fs::remove_file(&dst).await;
            return Ok(RestorePayload {
                backup_path: backup_path.to_string_lossy().to_string(),
                destination: destination.to_string_lossy().to_string(),
                success: false,
                copied_files,
                total_files,
                total_bytes,
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some(format!("{}: {}", rel, e)),
            });
        }
        copied_files += 1;
        copied_bytes += *size;
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

    Ok(RestorePayload {
        backup_path: backup_path.to_string_lossy().to_string(),
        destination: destination.to_string_lossy().to_string(),
        success: true,
        copied_files,
        total_files,
        total_bytes,
        duration_ms: started.elapsed().as_millis() as u64,
        error: None,
    })
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

async fn copy(src: &Path, dst: &Path) -> Result<()> {
    let src = long_path(src);
    let dst = long_path(dst);
    let (src, dst) = (src.as_path(), dst.as_path());
    let src_meta = fs::metadata(src).await.context("stat source")?;
    let mut r = fs::File::open(src).await.context("open source")?;
    // A read-only file already sitting in the chosen destination would make
    // File::create fail outright on Windows and abort the whole restore.
    clear_readonly(dst);
    let mut w = fs::File::create(dst).await.context("create destination")?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf).await.context("read source")?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).await.context("write destination")?;
    }
    w.flush().await?;
    // Durability: don't claim "restored" before the bytes hit disk.
    w.sync_all().await.context("sync destination")?;
    drop(w);
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

        copy(&src, &dst)
            .await
            .expect("a read-only file in the destination must not abort the restore");
        assert_eq!(std::fs::read(&dst).unwrap(), b"restored");

        crate::fsutil::clear_readonly(&dst);
        let _ = std::fs::remove_dir_all(&root);
    }
}
