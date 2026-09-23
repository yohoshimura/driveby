//! The desktop's accent colour, for the Adwaita style.
//!
//! libadwaita apps paint with the accent picked in GNOME Settings, read from
//! the settings portal and followed live. Driveby's Adwaita style does the
//! same on Linux: `system_accent` answers once at startup, and `watch`
//! forwards every later change as a `system-accent` event. There is no such
//! setting elsewhere, and the style keeps Adwaita's blue.
//!
//! The webview cannot find this out on its own: WebKitGTK answers CSS's
//! `AccentColor` with a fixed blue, whatever the desktop says.

use tauri::{AppHandle, Runtime};

/// The accent as a CSS hex colour, or `None` when the desktop has none.
#[tauri::command]
pub async fn system_accent() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        portal::read().await
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Follow the accent for the life of the app. A desktop without the portal
/// is not an error worth more than a debug line: the style simply keeps its
/// blue.
pub fn watch<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "linux")]
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = portal::watch(app).await {
                tracing::debug!("appearance: not following the accent colour: {}", e);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

/// The portal's three channels as `#rrggbb`. The spec reserves values
/// outside 0..=1 to mean "no accent set".
#[cfg(any(target_os = "linux", test))]
fn to_hex((r, g, b): (f64, f64, f64)) -> Option<String> {
    let channel = |v: f64| (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8);
    Some(format!("#{:02x}{:02x}{:02x}", channel(r)?, channel(g)?, channel(b)?))
}

#[cfg(target_os = "linux")]
mod portal {
    use futures_util::StreamExt;
    use tauri::{AppHandle, Emitter, Runtime};
    use zbus::zvariant::OwnedValue;
    use zbus::{Connection, Proxy};

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "accent-color";

    async fn settings(conn: &Connection) -> zbus::Result<Proxy<'static>> {
        Proxy::new(
            conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .await
    }

    fn rgb(value: OwnedValue) -> Option<(f64, f64, f64)> {
        <(f64, f64, f64)>::try_from(value).ok()
    }

    pub async fn read() -> Option<String> {
        let conn = Connection::session().await.ok()?;
        let value: OwnedValue = settings(&conn)
            .await
            .ok()?
            .call("ReadOne", &(NAMESPACE, KEY))
            .await
            .ok()?;
        super::to_hex(rgb(value)?)
    }

    pub async fn watch<R: Runtime>(app: AppHandle<R>) -> zbus::Result<()> {
        let conn = Connection::session().await?;
        let proxy = settings(&conn).await?;
        let mut changes = proxy.receive_signal("SettingChanged").await?;
        while let Some(message) = changes.next().await {
            let Ok((namespace, key, value)) =
                message.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace == NAMESPACE && key == KEY {
                let _ = app.emit("system-accent", rgb(value).and_then(super::to_hex));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::to_hex;

    #[test]
    fn channels_become_a_css_hex_colour() {
        // What GNOME answers for its red on Kali, and Adwaita's own blue.
        assert_eq!(to_hex((0.92549, 0.00392, 0.00392)).as_deref(), Some("#ec0101"));
        assert_eq!(to_hex((0.20784, 0.51765, 0.89412)).as_deref(), Some("#3584e4"));
    }

    /// Out of range is the spec's "no accent": the style must keep its blue
    /// rather than paint with a colour nobody chose.
    #[test]
    fn an_unset_accent_is_none() {
        assert_eq!(to_hex((-1.0, -1.0, -1.0)), None);
        assert_eq!(to_hex((0.5, 1.5, 0.5)), None);
    }
}
