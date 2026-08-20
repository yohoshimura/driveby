//! System tray icon and the close-to-tray flag.
//!
//! The scheduler runs inside this process, so with the window closed there
//! was nothing left to fire a scheduled backup — closing the window ended
//! the app. Keeping the process alive behind a tray icon is what makes a
//! schedule mean anything; the tray icon is how the user gets the window
//! back afterwards.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tracing::warn;

/// Settings the window-close handler needs. That handler is synchronous
/// and can't await a settings read, so `save_settings` mirrors the flag
/// into here as the user toggles it.
#[derive(Default)]
pub struct UiFlags {
    close_to_tray: AtomicBool,
}

impl UiFlags {
    pub fn set_close_to_tray(&self, value: bool) {
        self.close_to_tray.store(value, Ordering::Relaxed);
    }
    pub fn close_to_tray(&self) -> bool {
        self.close_to_tray.load(Ordering::Relaxed)
    }
}

/// Build the tray icon. Created unconditionally rather than only when
/// close-to-tray is on: it is the only route back to a hidden window, and
/// creating/destroying it as a setting changes is where the platform bugs
/// live.
// TODO(macOS): pair this with ActivationPolicy::Accessory so a
// tray-only session can drop the dock icon.
pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // Menu labels stay English for 1.6 — the tray menu would have to be
    // rebuilt on every language change to localize it.
    let open = MenuItem::with_id(app, "open", "Open driveby", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit driveby", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("default window icon".into())
        })?)
        .tooltip("driveby")
        .menu(&menu)
        // Without this a left click opens the menu on Windows and the
        // show-window handler below never runs.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        warn!("tray: no main window to show");
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}
