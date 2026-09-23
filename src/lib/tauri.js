import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { disable as disableAutostart, enable as enableAutostart, isEnabled as isAutostartEnabled } from '@tauri-apps/plugin-autostart';

export const bridge = {
  getSettings: () => invoke('get_settings'),
  saveSettings: (settings) => invoke('save_settings', { settings }),
  getTasks: () => invoke('get_tasks'),
  saveTasks: (tasks) => invoke('save_tasks', { tasks }),
  getHistory: () => invoke('get_history'),
  saveHistory: (history) => invoke('save_history', { history }),
  startBackup: (task, settings) => invoke('start_backup', { task, settings }),
  previewBackup: (task, settings) => invoke('preview_backup', { task, settings }),
  cancelPreview: () => invoke('cancel_preview'),
  cancelBackup: (taskId) => invoke('cancel_backup', { taskId }),
  restoreBackup: (backupPath, destination) => invoke('restore_backup', { backupPath, destination }),
  cancelRestore: () => invoke('cancel_restore'),
  revealLogsFolder: () => invoke('reveal_logs_folder'),

  selectDirectory: async (title) => {
    const res = await open({ directory: true, multiple: false, title });
    return typeof res === 'string' ? res : null;
  },

  revealFolder: async (folderPath) => {
    try {
      await revealItemInDir(folderPath);
      return { success: true };
    } catch (e) {
      return { success: false, error: String(e) };
    }
  },

  // Autostart is registered with the OS, not stored by us, so these read
  // and write the real registration.
  isAutostartEnabled: async () => {
    try {
      return await isAutostartEnabled();
    } catch {
      return false;
    }
  },

  setAutostart: async (want) => {
    try {
      if (want) await enableAutostart();
      else await disableAutostart();
      return true;
    } catch {
      return false;
    }
  },

  // `actions` become buttons on Windows and Linux, where Rust also acts on
  // the click: [{ kind: 'openFolder', label, path } | { kind: 'viewHistory', label, historyId }].
  // macOS shows the plain notification (see notification.rs).
  notify: async (title, body, actions = []) => {
    try {
      await invoke('notify', { title, body, actions });
    } catch {}
  },

  onBackupStarted: (cb) => listen('backup-started', (e) => cb(e.payload)),
  onBackupProgress: (cb) => listen('backup-progress', (e) => cb(e.payload)),
  onBackupComplete: (cb) => listen('backup-complete', (e) => cb(e.payload)),
  onRestoreProgress: (cb) => listen('restore-progress', (e) => cb(e.payload)),
  onTaskUpdated: (cb) => listen('task-updated', (e) => cb(e.payload)),
  // Emitted only when the answer changes, so the payload is the whole
  // picture: every task with at least one destination missing.
  onDestinationsStatus: (cb) => listen('destinations-status', (e) => cb(e.payload)),
  onDestinationMissing: (cb) => listen('destination-missing', (e) => cb(e.payload)),
  // A notification's "View in History" was clicked; the payload is the
  // history row's id. Rust has already brought the window up.
  onShowHistory: (cb) => listen('show-history', (e) => cb(e.payload)),
};
