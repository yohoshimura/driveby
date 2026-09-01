#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod fsutil;
mod glob;
mod persist;
mod preview;
mod ratelimit;
mod restore;
mod scheduler;
mod tray;

use backup::{BackupState, CompletePayload, Settings, Task};
use restore::RestoreState;
use serde_json::Value;
use std::path::PathBuf;
use tauri::Manager;
use tracing::info;
use tray::UiFlags;

fn default_settings() -> Value {
    serde_json::json!({
        "defaultDestination": "",
        "excludePatterns": "",
        "confirmBeforeBackup": true,
        "showNotifications": true,
        "accentColor": "blue",
        "theme": "system",
        "verify": false,
        "continueOnError": true,
        "preserveMtime": true,
        "parallelCopies": 4,
        "maxSpeedMbps": 0,
        "historyLimit": 1000,
        "closeToTray": false,
        "autostart": false,
        "checkUpdatesOnStart": true,
        "sidebarOpen": true,
        "lastView": "home",
        "language": "en"
    })
}

/// Whether the app should keep running when the window is closed. Read at
/// startup and re-read on every settings save, because the window-close
/// handler is synchronous and cannot await a file read.
fn close_to_tray_of(settings: &Value) -> bool {
    settings
        .get("closeToTray")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// The speed ceiling in bytes per second, read through `Settings` rather than
/// off the JSON directly so that the awkward values a hand-edited file can
/// hold — negative, NaN, infinity — are all rejected the one way they are
/// rejected everywhere else.
fn max_speed_bytes_of(settings: &Value) -> u64 {
    serde_json::from_value::<Settings>(settings.clone())
        .unwrap_or_default()
        .max_speed_bytes()
}

/// Push the ceiling into the process-wide limiter.
///
/// `run_backup` sets it too, from the settings it was handed — but that only
/// covers a ceiling chosen *before* a run starts. The setting exists so the
/// machine stays usable while a backup runs, which means the moment it is
/// most likely to be changed is halfway through one. Mirrored here for the
/// same reason `closeToTray` is: the live state has to follow the file.
fn apply_speed_ceiling(settings: &Value) {
    ratelimit::shared().set_rate(max_speed_bytes_of(settings));
}

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let path = persist::data_path(&app, "settings.json")?;
    Ok(persist::read_json_or(&path, default_settings()).await)
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    flags: tauri::State<'_, UiFlags>,
    settings: Value,
) -> Result<(), String> {
    let path = persist::data_path(&app, "settings.json")?;
    flags.set_close_to_tray(close_to_tray_of(&settings));
    apply_speed_ceiling(&settings);
    persist::write_json_atomic(&path, &settings)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_tasks(app: tauri::AppHandle) -> Result<Value, String> {
    let path = persist::data_path(&app, "tasks.json")?;
    Ok(persist::read_json_or(&path, Value::Array(vec![])).await)
}

/// Carry forward a `lastBackup` the incoming list has not caught up with.
///
/// The lock cannot span the read half of this read-modify-write: `get_tasks`
/// answered the webview when it mounted, and the edit happened in React. What
/// it *can* do is refuse to let a stale copy of the one field Rust owns
/// overwrite a fresher one. `update_last_backup` is the only writer of
/// tasks.json on this side and `lastBackup` is all it writes, so declining a
/// value older than the one on disk closes the whole window — without a
/// protocol change, and without touching any field the user edits.
///
/// RFC3339 in UTC, which is what `update_last_backup` emits, orders correctly
/// as a plain string: fixed-width fields, most significant first. A missing or
/// differently shaped value reads as `""` and therefore only ever loses to a
/// real timestamp, never overwrites one.
///
/// A task the user deleted is simply absent from `incoming`, and nothing here
/// puts it back.
fn keep_newer_last_backup(incoming: &mut Value, on_disk: &Value) {
    let Some(stored) = on_disk.as_array() else {
        return;
    };
    let Some(items) = incoming.as_array_mut() else {
        return;
    };
    for task in items {
        let Some(obj) = task.as_object_mut() else {
            continue;
        };
        let Some(id) = obj.get("id").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        let Some(disk_ts) = stored
            .iter()
            .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
            .and_then(|t| t.get("lastBackup"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let ours = obj.get("lastBackup").and_then(|v| v.as_str()).unwrap_or("");
        if ours < disk_ts {
            obj.insert("lastBackup".into(), Value::String(disk_ts.to_string()));
        }
    }
}

#[tauri::command]
async fn save_tasks(app: tauri::AppHandle, tasks: Value) -> Result<(), String> {
    let path = persist::data_path(&app, "tasks.json")?;
    let mut tasks = tasks;
    // Hold the same lock the backup pipeline uses, so two writes cannot
    // interleave — and re-read inside it, so the write cannot carry a
    // lastBackup that went stale while the user had the list open.
    persist::with_tasks_lock(|| async {
        let on_disk: Value = persist::read_json_or(&path, Value::Array(vec![])).await;
        keep_newer_last_backup(&mut tasks, &on_disk);
        persist::write_json_atomic(&path, &tasks)
            .await
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn get_history(app: tauri::AppHandle) -> Result<Value, String> {
    let path = persist::data_path(&app, "history.json")?;
    Ok(persist::read_json_or(&path, Value::Array(vec![])).await)
}

#[tauri::command]
async fn save_history(app: tauri::AppHandle, history: Value) -> Result<(), String> {
    let path = persist::data_path(&app, "history.json")?;
    persist::with_history_lock(|| async {
        persist::write_json_atomic(&path, &history)
            .await
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn start_backup(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackupState>,
    task: Task,
    settings: Settings,
) -> Result<CompletePayload, String> {
    backup::run_backup(&app, &state, task, settings)
        .await
        .map_err(|e| e.to_string())
}

/// What `start_backup` would do, without doing it. Cancellable, because on
/// a large tree it is a full source walk plus a stat of every counterpart,
/// and the user is sitting in front of a modal dialog while it runs.
#[tauri::command]
async fn preview_backup(
    state: tauri::State<'_, preview::PreviewState>,
    task: Task,
    settings: Settings,
) -> Result<preview::PreviewPayload, String> {
    preview::plan_backup(&state, task, settings)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cancel_preview(state: tauri::State<'_, preview::PreviewState>) -> Result<(), String> {
    state.cancel();
    Ok(())
}

#[tauri::command]
async fn cancel_backup(
    state: tauri::State<'_, BackupState>,
    task_id: String,
) -> Result<(), String> {
    state.cancel(&task_id);
    Ok(())
}

#[tauri::command]
async fn restore_backup(
    app: tauri::AppHandle,
    state: tauri::State<'_, RestoreState>,
    backup_path: String,
    destination: String,
) -> Result<restore::RestorePayload, String> {
    restore::run_restore(
        &app,
        &state,
        PathBuf::from(backup_path),
        PathBuf::from(destination),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cancel_restore(state: tauri::State<'_, RestoreState>) -> Result<(), String> {
    state.cancel();
    Ok(())
}

#[tauri::command]
fn reveal_logs_folder(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

fn setup_logging(app: &tauri::AppHandle) {
    let log_dir = match app.path().app_log_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "driveby.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard — lives for program lifetime.
    std::mem::forget(guard);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .try_init();
}

fn main() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // Launched by the OS at login, the app should come up in the
            // background rather than throwing a window at the user.
            Some(vec!["--hidden"]),
        ))
        .manage(BackupState::default())
        .manage(RestoreState::default())
        .manage(preview::PreviewState::default())
        .manage(UiFlags::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let flags = window.state::<UiFlags>();
                if flags.close_to_tray() {
                    // Keep the process — and with it the scheduler — alive.
                    api.prevent_close();
                    let _ = window.hide();
                    // On macOS the Dock tile outlives the window it stands
                    // for, so drop it: the tray icon is now the only way back.
                    tray::set_dock_visible(window.app_handle(), false);
                }
            }
        })
        .setup(|app| {
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
            }
            setup_logging(app.handle());
            info!("Driveby {} starting", env!("CARGO_PKG_VERSION"));

            if let Err(e) = tray::setup(app.handle()) {
                tracing::warn!("could not create tray icon: {}", e);
            }

            // Seed the close-to-tray flag from disk; save_settings keeps it
            // in step from then on.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(path) = persist::data_path(&handle, "settings.json") {
                    let settings = persist::read_json_or(&path, default_settings()).await;
                    handle
                        .state::<UiFlags>()
                        .set_close_to_tray(close_to_tray_of(&settings));
                    apply_speed_ceiling(&settings);
                }
            });

            // Autostart passes --hidden: come up in the tray only. Launched
            // at login there is no window at all, so a Dock tile would be
            // there purely to be clicked and do nothing.
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
                tray::set_dock_visible(app.handle(), false);
            }

            scheduler::spawn(app.handle().clone());
            Ok(())
        });

    builder = builder.invoke_handler(tauri::generate_handler![
        get_settings,
        save_settings,
        get_tasks,
        save_tasks,
        get_history,
        save_history,
        start_backup,
        preview_backup,
        cancel_preview,
        cancel_backup,
        restore_backup,
        cancel_restore,
        reveal_logs_folder,
    ]);

    // `main()` is the only place we let the runtime own the process: a
    // failure here means the Tauri builder couldn't bring up the webview /
    // event loop at all. There's nothing useful to recover to, so panic
    // with a message that tells operators where to look first.
    builder
        .run(tauri::generate_context!())
        .expect("Tauri runtime failed to start (check logs in app_log_dir/driveby.log)");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ceiling is read out of the same JSON the frontend writes, so the
    /// camelCase key has to survive the round trip. A rename mismatch here
    /// would not fail loudly — it would silently read `None` and mean "no
    /// ceiling" for ever, which is exactly the setting being off.
    #[test]
    fn the_ceiling_is_read_from_the_key_the_ui_writes() {
        let settings = serde_json::json!({ "maxSpeedMbps": 5.0 });
        assert_eq!(max_speed_bytes_of(&settings), 5 * 1024 * 1024);
    }

    /// Every way of saying "no ceiling", including the ones a hand-edited
    /// settings.json can produce. `Settings::max_speed_bytes` owns these
    /// rules; this pins that the JSON path really goes through it rather
    /// than re-deriving them.
    #[test]
    fn absent_and_absurd_ceilings_all_mean_no_ceiling() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({ "maxSpeedMbps": 0 }),
            serde_json::json!({ "maxSpeedMbps": -5.0 }),
            serde_json::json!({ "maxSpeedMbps": null }),
        ] {
            assert_eq!(max_speed_bytes_of(&value), 0, "for {}", value);
        }
    }

    /// A settings file whose types are wrong must not read as a tiny
    /// ceiling that would stall every copy — it reads as no ceiling.
    #[test]
    fn an_unparseable_settings_object_leaves_the_ceiling_off() {
        let settings = serde_json::json!({ "maxSpeedMbps": "fast" });
        assert_eq!(max_speed_bytes_of(&settings), 0);
    }

    #[test]
    fn close_to_tray_defaults_to_off() {
        assert!(!close_to_tray_of(&serde_json::json!({})));
        assert!(close_to_tray_of(&serde_json::json!({ "closeToTray": true })));
    }

    /// The window TASKS_LOCK never covered. A scheduled run stamps
    /// `lastBackup` while the user has the task list open; the list the
    /// webview then saves was read before that stamp, and the write used to
    /// put the old value back — so the scheduler ran the task again on the
    /// next tick. The user's own edits to every other field still win.
    #[test]
    fn a_save_cannot_put_back_a_last_backup_older_than_the_one_on_disk() {
        let mut incoming = serde_json::json!([
            { "id": "a", "name": "renamed by the user", "lastBackup": "2026-08-30T10:00:00+00:00" },
        ]);
        let on_disk = serde_json::json!([
            { "id": "a", "name": "old name", "lastBackup": "2026-08-31T09:00:00+00:00" },
        ]);
        keep_newer_last_backup(&mut incoming, &on_disk);
        assert_eq!(incoming[0]["lastBackup"], "2026-08-31T09:00:00+00:00");
        assert_eq!(
            incoming[0]["name"], "renamed by the user",
            "the user's edit still wins"
        );
    }

    /// Absent, null, and a value the incoming list made *newer* all behave.
    /// A task created by this very save has no counterpart on disk and must
    /// be left exactly as sent, `lastBackup: null` included.
    #[test]
    fn a_newer_or_absent_counterpart_leaves_the_incoming_value_alone() {
        let mut incoming = serde_json::json!([
            { "id": "a", "lastBackup": null },
            { "id": "b", "lastBackup": "2026-08-31T12:00:00+00:00" },
            { "id": "new", "lastBackup": null },
        ]);
        let on_disk = serde_json::json!([
            { "id": "a", "lastBackup": "2026-08-31T09:00:00+00:00" },
            { "id": "b", "lastBackup": "2026-08-31T08:00:00+00:00" },
        ]);
        keep_newer_last_backup(&mut incoming, &on_disk);
        assert_eq!(
            incoming[0]["lastBackup"], "2026-08-31T09:00:00+00:00",
            "null loses to a real stamp"
        );
        assert_eq!(
            incoming[1]["lastBackup"], "2026-08-31T12:00:00+00:00",
            "the newer one stands"
        );
        assert_eq!(
            incoming[2]["lastBackup"],
            Value::Null,
            "a task with no counterpart on disk is untouched"
        );
    }

    /// A task the user deleted must stay deleted: it is simply not in the
    /// incoming list, and nothing here puts it back.
    #[test]
    fn a_deleted_task_is_not_resurrected_from_disk() {
        let mut incoming = serde_json::json!([{ "id": "kept", "lastBackup": null }]);
        let on_disk = serde_json::json!([
            { "id": "kept", "lastBackup": "2026-08-31T09:00:00+00:00" },
            { "id": "deleted", "lastBackup": "2026-08-31T09:30:00+00:00" },
        ]);
        keep_newer_last_backup(&mut incoming, &on_disk);
        assert_eq!(incoming.as_array().unwrap().len(), 1);
        assert_eq!(incoming[0]["id"], "kept");
    }
}
