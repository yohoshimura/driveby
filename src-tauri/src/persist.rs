use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub async fn read_json_or<T: DeserializeOwned>(path: &Path, fallback: T) -> T {
    match fs::read_to_string(path).await {
        Ok(data) => serde_json::from_str(&data).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

/// Serial for scratch filenames. A fixed name like `history.json.tmp` is
/// shared by every concurrent caller: the second File::create truncates the
/// first writer's file, the second renames it into place, and the first then
/// keeps writing at its own offset into what is now the live document. The
/// result is interleaved JSON that fails to parse on the next launch, and
/// read_json_or quietly falls back to an empty list — the whole history gone
/// without a message (#R5).
static NEXT_TMP: AtomicU64 = AtomicU64::new(0);

pub async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_vec_pretty(value)?;
    // Unique per call, so the rename is the only step that can race — and a
    // racing rename simply means one complete document replaces another
    // complete document, which is the worst it can be.
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    let written = async {
        let mut f = fs::File::create(&tmp).await?;
        f.write_all(&json).await?;
        f.sync_all().await?;
        drop(f);
        fs::rename(&tmp, path).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if written.is_err() {
        // Don't leave scratch files behind for a failed write.
        let _ = fs::remove_file(&tmp).await;
    }
    written
}

// Shared lock for any tasks.json read-modify-write. Both the JS-driven
// save_tasks command and the Rust-driven update_last_backup must hold this
// for their full critical section, otherwise the scheduler's per-task
// lastBackup write can clobber a user edit that landed between the read and
// the write (#7).
static TASKS_LOCK: Mutex<()> = Mutex::const_new(());

pub async fn with_tasks_lock<F, Fut, T>(f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let _guard = TASKS_LOCK.lock().await;
    f().await
}

// history.json needs the same treatment: two scheduled tasks finishing
// milliseconds apart have the frontend firing save_history twice, and
// without this the two writes interleave at the rename.
static HISTORY_LOCK: Mutex<()> = Mutex::const_new(());

pub async fn with_history_lock<F, Fut, T>(f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let _guard = HISTORY_LOCK.lock().await;
    f().await
}
