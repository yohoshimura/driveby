//! Desktop notifications that act on a click.
//!
//! tauri-plugin-notification only *shows* a notification on desktop: its
//! actions and click events are mobile-only. The APIs it sits on can do both
//! on Windows (toasts) and on Linux (the freedesktop notification spec), so
//! there the notification is built here, with buttons that work even while
//! the window is hidden in the tray. macOS keeps the plugin's plain
//! notification: the API it uses there shows buttons only to users who switch
//! Driveby to the "Alerts" style, and its replacement needs a signed bundle.

use serde::Deserialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
use toast as platform;
#[cfg(target_os = "linux")]
use xdg as platform;

/// A button on the notification, as the frontend sends it. The label comes
/// from there too: the reader's language lives in the webview.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub enum Action {
    OpenFolder { label: String, path: String },
    ViewHistory { label: String, history_id: String },
}

pub fn show<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    body: &str,
    actions: Vec<Action>,
) -> Result<(), String> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        match platform::show(app, title, body, actions) {
            Ok(()) => return Ok(()),
            // Losing the buttons is better than losing the news.
            Err(e) => tracing::warn!("notification: failed with actions, sending a plain one: {}", e),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = actions;

    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

/// What a click does, whichever platform reported it.
#[cfg(any(windows, target_os = "linux"))]
mod click {
    use super::Action;
    use tauri::{AppHandle, Emitter, Runtime};
    use tracing::warn;

    impl Action {
        /// What the platform hands back when this button is clicked.
        pub(super) fn id(&self) -> &'static str {
            match self {
                Action::OpenFolder { .. } => "open-folder",
                Action::ViewHistory { .. } => "view-history",
            }
        }

        pub(super) fn label(&self) -> &str {
            match self {
                Action::OpenFolder { label, .. } | Action::ViewHistory { label, .. } => label,
            }
        }
    }

    /// Which action a click asked for. A button names its own; `None` is a
    /// click on the notification itself, which means "show me" — the History
    /// row when there is one.
    pub(super) fn chosen<'a>(actions: &'a [Action], argument: Option<&str>) -> Option<&'a Action> {
        match argument {
            Some(id) => actions.iter().find(|a| a.id() == id),
            None => actions.iter().find(|a| matches!(a, Action::ViewHistory { .. })),
        }
    }

    pub(super) fn run<R: Runtime>(app: &AppHandle<R>, action: Option<&Action>) {
        match action {
            Some(Action::OpenFolder { path, .. }) => {
                // The free function rather than `Opener::open_path`: it stats
                // the path first, so a drive unplugged since the run ends up
                // in the log instead of in front of the file manager.
                if let Err(e) = tauri_plugin_opener::open_path(path, None::<&str>) {
                    warn!(path = %path, "notification: could not open destination: {}", e);
                }
            }
            Some(Action::ViewHistory { history_id, .. }) => {
                crate::tray::show_main_window(app);
                let _ = app.emit("show-history", history_id);
            }
            None => crate::tray::show_main_window(app),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn both() -> Vec<Action> {
            vec![
                Action::OpenFolder { label: "Open".into(), path: "/mnt/dst".into() },
                Action::ViewHistory { label: "History".into(), history_id: "run-1".into() },
            ]
        }

        #[test]
        fn a_button_click_picks_its_own_action() {
            let actions = both();
            assert!(matches!(
                chosen(&actions, Some("open-folder")),
                Some(Action::OpenFolder { .. })
            ));
            assert!(matches!(
                chosen(&actions, Some("view-history")),
                Some(Action::ViewHistory { .. })
            ));
        }

        /// The body of the notification is not a button: it goes to the
        /// History row when the notification has one, and only raises the
        /// window when not.
        #[test]
        fn a_click_on_the_body_goes_to_history_when_it_can() {
            let actions = both();
            assert!(matches!(chosen(&actions, None), Some(Action::ViewHistory { .. })));
            let folder_only = vec![actions[0].clone()];
            assert!(chosen(&folder_only, None).is_none());
            assert!(chosen(&[], None).is_none());
        }

        #[test]
        fn an_unknown_argument_picks_nothing() {
            assert!(chosen(&both(), Some("something-else")).is_none());
        }
    }
}

#[cfg(windows)]
mod toast {
    use super::click::{chosen, run};
    use super::Action;
    use std::path::Path;
    use tauri::{AppHandle, Runtime};
    use tauri_winrt_notification::{Duration, Toast};

    pub fn show<R: Runtime>(
        app: &AppHandle<R>,
        title: &str,
        body: &str,
        actions: Vec<Action>,
    ) -> Result<(), String> {
        let mut toast = Toast::new(&app_id(app))
            .title(title)
            .text1(body)
            // What the plugin sent until now: a short toast, and silent.
            .duration(Duration::Short)
            .sound(None);
        for action in &actions {
            toast = toast.add_button(&escape(action.label()), action.id());
        }
        let app = app.clone();
        toast
            .on_activated(move |argument| {
                run(&app, chosen(&actions, argument.as_deref()));
                Ok(())
            })
            .show()
            .map_err(|e| e.to_string())
    }

    /// The AppUserModelID the toast is filed under, picked the way the
    /// plugin picks it: only an installed build has the Start menu shortcut
    /// that registers our identifier, so a build run straight out of
    /// `target/` borrows PowerShell's.
    fn app_id<R: Runtime>(app: &AppHandle<R>) -> String {
        let from_target = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .is_some_and(|dir| dir.ends_with("target/debug") || dir.ends_with("target/release"));
        if from_target {
            Toast::POWERSHELL_APP_ID.to_string()
        } else {
            app.config().identifier.clone()
        }
    }

    /// `add_button` pastes its label into a single-quoted XML attribute as
    /// is — unlike `title` and `text1`, which escape theirs — so an
    /// apostrophe or an ampersand in a label would leave the toast XML
    /// unparseable.
    fn escape(label: &str) -> String {
        label
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "&apos;")
            .replace('"', "&quot;")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_label_cannot_break_out_of_its_attribute() {
            assert_eq!(
                escape(r#"Tom's <b> & "x""#),
                "Tom&apos;s &lt;b&gt; &amp; &quot;x&quot;"
            );
            // The typographic apostrophe the French strings use is not markup.
            assert_eq!(escape("Voir dans l’historique"), "Voir dans l’historique");
        }
    }
}

#[cfg(target_os = "linux")]
mod xdg {
    use super::click::{chosen, run};
    use super::Action;
    use notify_rust::Notification;
    use tauri::{AppHandle, Runtime};

    /// The action key the freedesktop spec reserves for a click on the
    /// notification itself. GNOME and Plasma draw no button for it.
    const BODY: &str = "default";
    /// What notify-rust reports when the notification goes away unanswered.
    const CLOSED: &str = "__closed";

    pub fn show<R: Runtime>(
        app: &AppHandle<R>,
        title: &str,
        body: &str,
        actions: Vec<Action>,
    ) -> Result<(), String> {
        let mut notification = Notification::new();
        // What the plugin sends on Linux, plus the actions.
        notification.summary(title).body(body).auto_icon();
        // The label only shows on servers that list every action in a menu,
        // dunst among them.
        notification.action(BODY, title);
        for action in &actions {
            notification.action(action.id(), action.label());
        }
        let handle = notification.show().map_err(|e| e.to_string())?;

        // `wait_for_action` blocks until the notification is answered or
        // closed, which can be long after the run: GNOME keeps it in the
        // message tray until it is dismissed. One parked thread per
        // notification, and there is at most one notification per run.
        let app = app.clone();
        std::thread::Builder::new()
            .name("notification".into())
            .spawn(move || {
                handle.wait_for_action(|key| match key {
                    CLOSED => {}
                    BODY => run(&app, chosen(&actions, None)),
                    id => run(&app, chosen(&actions, Some(id))),
                })
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `bridge.notify` sends from AppContext. A rename on either
    /// side would fail the whole `notify` call — the notification with it,
    /// buttons or not.
    #[test]
    fn the_actions_the_frontend_sends_deserialize() {
        let actions: Vec<Action> = serde_json::from_value(serde_json::json!([
            { "kind": "openFolder", "label": "Open destination folder", "path": "D:\\Backups" },
            { "kind": "viewHistory", "label": "View in History", "historyId": "4f1c" },
        ]))
        .unwrap();
        assert!(matches!(
            &actions[0],
            Action::OpenFolder { label, path } if label == "Open destination folder" && path == "D:\\Backups"
        ));
        assert!(matches!(
            &actions[1],
            Action::ViewHistory { history_id, .. } if history_id == "4f1c"
        ));
    }
}
