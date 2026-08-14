use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub async fn read_json_or<T: DeserializeOwned>(path: &Path, fallback: T) -> T {
    match fs::read_to_string(path).await {
        Ok(data) => serde_json::from_str(&data).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

pub async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::File::create(&tmp).await?;
    f.write_all(&json).await?;
    f.sync_all().await?;
    drop(f);
    fs::rename(&tmp, path).await?;
    Ok(())
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
