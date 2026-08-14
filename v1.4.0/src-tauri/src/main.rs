#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod fsutil;
mod glob;
mod persist;
mod restore;
mod scheduler;

use backup::{BackupState, CompletePayload, Settings, Task};
use serde_json::Value;
use std::path::PathBuf;
use tauri::Manager;
use tracing::info;

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
        "sidebarOpen": true,
        "lastView": "home",
        "language": "en"
    })
}

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let path = data_path(&app, "settings.json")?;
    Ok(persist::read_json_or(&path, default_settings()).await)
}

#[tauri::command]
async fn save_settings(app: tauri::AppHandle, settings: Value) -> Result<(), String> {
    let path = data_path(&app, "settings.json")?;
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
    backup_path: String,
    destination: String,
) -> Result<restore::RestorePayload, String> {
    restore::restore(&app, PathBuf::from(backup_path), PathBuf::from(destination))
        .await
        .map_err(|e| e.to_string())
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
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(BackupState::default())
        .setup(|app| {
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
            }
            setup_logging(app.handle());
            info!("driveby 1.4 starting");
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
