import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

// Thin wrapper so components never touch the plugin API directly and a
// build with the updater switched off degrades to "no update found"
// instead of throwing into the UI.

export async function checkForUpdate() {
  try {
    const update = await check();
    if (!update) return { available: false };
    return { available: true, version: update.version, notes: update.body, update };
  } catch (e) {
    return { available: false, error: String(e) };
  }
}

// Downloads and installs, then restarts into the new version. Never called
// on its own — the user asks for it from Settings.
export async function installUpdate(update) {
  await update.downloadAndInstall();
  await relaunch();
}
