import { describe, expect, test } from 'vitest';
import { platformOf, resolveUiStyle, UI_STYLES, DEFAULT_UI_STYLE } from '../uiStyle';

// What each webview actually reports: WebView2 on Windows, WebKitGTK on
// Linux, WKWebView on macOS.
const WEBVIEW2 = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0';
const WEBKITGTK = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15';
const WKWEBVIEW = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';

describe('platformOf', () => {
  test('names the OS each webview reports', () => {
    expect(platformOf(WEBVIEW2)).toBe('windows');
    expect(platformOf(WEBKITGTK)).toBe('linux');
    expect(platformOf(WKWEBVIEW)).toBe('macos');
  });

  // No Linux-only rule may reach a platform we cannot name.
  test('anything it does not recognise is not Linux', () => {
    expect(platformOf('')).toBe('macos');
    expect(platformOf(undefined)).toBe('macos');
  });
});

describe('resolveUiStyle', () => {
  test('auto follows the platform the webview reports', () => {
    expect(resolveUiStyle('auto', WEBVIEW2)).toBe('windows');
    expect(resolveUiStyle('auto', WEBKITGTK)).toBe('gnome');
    expect(resolveUiStyle('auto', WKWEBVIEW)).toBe('ios');
  });

  test('auto falls back to ios on a platform it does not recognise', () => {
    expect(resolveUiStyle('auto', '')).toBe('ios');
    expect(resolveUiStyle('auto', undefined)).toBe('ios');
  });

  test('an explicit choice wins over the platform', () => {
    expect(resolveUiStyle('ios', WEBVIEW2)).toBe('ios');
    expect(resolveUiStyle('gnome', WEBVIEW2)).toBe('gnome');
    expect(resolveUiStyle('windows', WEBKITGTK)).toBe('windows');
  });

  test('a value it does not know is treated as auto', () => {
    // A hand-edited settings.json, or one written by a later version that
    // added a style this build has never heard of.
    expect(resolveUiStyle('fluent2', WEBVIEW2)).toBe('windows');
    expect(resolveUiStyle(undefined, WEBKITGTK)).toBe('gnome');
  });

  test('the picker lists auto first and it is the default', () => {
    expect(UI_STYLES[0]).toBe('auto');
    expect(DEFAULT_UI_STYLE).toBe('auto');
  });
});
