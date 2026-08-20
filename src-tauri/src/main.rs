#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod fsutil;
mod glob;
mod persist;
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

fn data_path(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {}", e))?;
    Ok(dir.join(name))
}

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

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let path = data_path(&app, "settings.json")?;
    Ok(persist::read_json_or(&path, default_settings()).await)
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    flags: tauri::State<'_, UiFlags>,
    settings: Value,
) -> Result<(), String> {
    let path = data_path(&app, "settings.json")?;
    flags.set_close_to_tray(close_to_tray_of(&settings));
    persist::write_json_atomic(&path, &settings)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_tasks(app: tauri::AppHandle) -> Result<Value, String> {
    let path = data_path(&app, "tasks.json")?;
    Ok(persist::read_json_or(&path, Value::Array(vec![])).await)
}

#[tauri::command]
async fn save_tasks(app: tauri::AppHandle, tasks: Value) -> Result<(), String> {
    let path = data_path(&app, "tasks.json")?;
    // Hold the same lock the backup pipeline uses so a scheduler-driven
    // lastBackup write can't land between the JS read-modify-write and
    // clobber a user edit (#7).
    persist::with_tasks_lock(|| async {
        persist::write_json_atomic(&path, &tasks)
            .await
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn get_history(app: tauri::AppHandle) -> Result<Value, String> {
    let path = data_path(&app, "history.json")?;
    Ok(persist::read_json_or(&path, Value::Array(vec![])).await)
}

#[tauri::command]
async fn save_history(app: tauri::AppHandle, history: Value) -> Result<(), String> {
    let path = data_path(&app, "history.json")?;
    persist::write_json_atomic(&path, &history)
        .await
        .map_err(|e| e.to_string())
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
        .manage(UiFlags::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let flags = window.state::<UiFlags>();
                if flags.close_to_tray() {
                    // Keep the process — and with it the scheduler — alive.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
            }
            setup_logging(app.handle());
            info!("driveby {} starting", env!("CARGO_PKG_VERSION"));

            if let Err(e) = tray::setup(app.handle()) {
                tracing::warn!("could not create tray icon: {}", e);
            }

            // Seed the close-to-tray flag from disk; save_settings keeps it
            // in step from then on.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(path) = data_path(&handle, "settings.json") {
                    let settings = persist::read_json_or(&path, default_settings()).await;
                    handle
                        .state::<UiFlags>()
                        .set_close_to_tray(close_to_tray_of(&settings));
                }
            });

            // Autostart passes --hidden: come up in the tray only.
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
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
