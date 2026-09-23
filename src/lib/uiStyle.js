// The visual language the window is drawn in, independent of light/dark.
// 'auto' matches the host OS so the app looks at home wherever it runs; the
// other values pin one style regardless of platform.

export const UI_STYLES = ['auto', 'ios', 'windows', 'gnome'];
export const DEFAULT_UI_STYLE = 'auto';

/// The OS the webview runs on: 'windows', 'linux', or 'macos' for anything
/// else. Read from the user agent rather than asked of Rust: it is
/// synchronous, so the first paint already carries the right style instead
/// of flashing the default one first. Every webview Tauri uses names its OS
/// there — WebView2 says "Windows", WebKitGTK says "Linux" (or "X11").
export function platformOf(userAgent = '') {
  if (/Windows/i.test(userAgent)) return 'windows';
  if (/Linux|X11/i.test(userAgent) && !/Android/i.test(userAgent)) return 'linux';
  return 'macos';
}

const PLATFORM_STYLE = { windows: 'windows', linux: 'gnome', macos: 'ios' };

/// Turns the stored preference into the value set on `data-style`. Anything
/// that is not a concrete style — 'auto', a missing key, a value from a
/// newer version — is resolved against the platform.
export function resolveUiStyle(preference, userAgent) {
  if (preference !== 'auto' && UI_STYLES.includes(preference)) return preference;
  return PLATFORM_STYLE[platformOf(userAgent)];
}
