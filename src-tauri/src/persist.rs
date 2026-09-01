use anyhow::Result;
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::warn;

/// Where a data document lives.
///
/// The one answer, so that no two modules can quietly disagree about which
/// directory `tasks.json` is in — this was written three ways, and the three
/// did not even agree on how to spell failure. Callers that must keep working
/// without it still can: `.ok()` reads exactly as the `Option` version did.
pub fn data_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    name: &str,
) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {}", e))?;
    Ok(dir.join(name))
}

/// What loading a JSON document found.
///
/// The distinction `read_json_or` could not make on its own: a document that
/// is not there yet and one that is there and unusable both produced the
/// caller's default, silently and identically. That is the right answer for
/// *rendering* — an empty list either way — and the wrong one for anything
/// that then writes the document back, or reasons about what is missing from
/// it. The scheduler did the latter and wiped every task's clock.
pub enum Loaded<T> {
    /// Parsed cleanly.
    Ok(T),
    /// Nothing on disk yet: a first launch, or a document never saved.
    Missing,
    /// Something is on disk that could not be used. An unparseable file has
    /// been moved aside first, so it stays recoverable.
    Damaged,
}

/// Move an unusable document aside before the caller's default can be written
/// over it.
///
/// This is what turned #R5 from an inconvenience into data loss. The empty
/// fallback rendered as "no tasks", the next save wrote that over the only
/// copy, and the evidence went with it — so the file is preserved *before* the
/// fallback is handed out, not after somebody notices.
async fn quarantine(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(format!(".corrupt-{}", Utc::now().format("%Y%m%dT%H%M%S")));
    let aside = path.with_file_name(name);
    match fs::rename(path, &aside).await {
        Ok(()) => Some(aside),
        Err(e) => {
            warn!("could not set aside {}: {}", path.display(), e);
            None
        }
    }
}

pub async fn load_json<T: DeserializeOwned>(path: &Path) -> Loaded<T> {
    let data = match fs::read_to_string(path).await {
        Ok(data) => data,
        // Absent is the ordinary case on a first launch and says nothing is
        // wrong. Any other IO error — a lock, a permission, a bad sector — is
        // damage: it is *not* quarantined, because a file we could not read is
        // not a file we know to be bad.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(e) => {
            warn!("could not read {}: {}", path.display(), e);
            return Loaded::Damaged;
        }
    };
    match serde_json::from_str(&data) {
        Ok(value) => Loaded::Ok(value),
        Err(e) => {
            match quarantine(path).await {
                Some(aside) => warn!(
                    "{} could not be parsed ({}); kept as {} and starting from the default",
                    path.display(),
                    e,
                    aside.display()
                ),
                None => warn!(
                    "{} could not be parsed ({}) and could not be set aside",
                    path.display(),
                    e
                ),
            }
            Loaded::Damaged
        }
    }
}

pub async fn read_json_or<T: DeserializeOwned>(path: &Path, fallback: T) -> T {
    match load_json(path).await {
        Loaded::Ok(value) => value,
        Loaded::Missing | Loaded::Damaged => fallback,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("driveby-persist-test-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_document_that_was_never_saved_reads_as_missing() {
        let dir = scratch("missing");
        let loaded: Loaded<Vec<String>> = load_json(&dir.join("tasks.json")).await;
        assert!(matches!(loaded, Loaded::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one that mattered. An unparseable document used to be handed back
    /// as the caller's default with no log line and no copy kept — the next
    /// save then wrote that default over the only evidence. It is now set
    /// aside first, and reported as damaged so callers that would overwrite
    /// it, or reason about what is missing from it, can decline.
    #[tokio::test]
    async fn a_corrupt_document_is_kept_aside_and_reported_as_damaged() {
        let dir = scratch("corrupt");
        let path = dir.join("tasks.json");
        std::fs::write(&path, b"[{ truncated mid-w").unwrap();

        let loaded: Loaded<Vec<String>> = load_json(&path).await;
        assert!(matches!(loaded, Loaded::Damaged));

        assert!(!path.exists(), "the bad file must not be left where a save would land on it");
        let kept: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("tasks.json.corrupt-"))
            .collect();
        assert_eq!(kept.len(), 1, "expected one preserved copy, found {:?}", kept);

        assert_eq!(
            std::fs::read(dir.join(&kept[0])).unwrap(),
            b"[{ truncated mid-w",
            "the copy has to be the original bytes, not a rewrite"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `read_json_or` keeps its old signature and its old answer; what changed
    /// is that the file behind it survives.
    #[tokio::test]
    async fn read_json_or_still_falls_back_but_no_longer_loses_the_file() {
        let dir = scratch("fallback");
        let path = dir.join("history.json");
        std::fs::write(&path, b"not json at all").unwrap();

        let value: Vec<String> = read_json_or(&path, vec!["fallback".to_string()]).await;
        assert_eq!(value, vec!["fallback".to_string()]);
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_good_document_round_trips_untouched() {
        let dir = scratch("roundtrip");
        let path = dir.join("settings.json");
        write_json_atomic(&path, &vec!["a".to_string(), "b".to_string()]).await.unwrap();

        let loaded: Loaded<Vec<String>> = load_json(&path).await;
        match loaded {
            Loaded::Ok(v) => assert_eq!(v, vec!["a".to_string(), "b".to_string()]),
            _ => panic!("a document we just wrote must load"),
        }
        assert!(path.exists(), "a good document is never set aside");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
