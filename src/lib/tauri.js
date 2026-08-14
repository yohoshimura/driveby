import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';

export const bridge = {
  getSettings: () => invoke('get_settings'),
  saveSettings: (settings) => invoke('save_settings', { settings }),
  getTasks: () => invoke('get_tasks'),
  saveTasks: (tasks) => invoke('save_tasks', { tasks }),
  getHistory: () => invoke('get_history'),
  saveHistory: (history) => invoke('save_history', { history }),
  startBackup: (task, settings) => invoke('start_backup', { task, settings }),
  cancelBackup: (taskId) => invoke('cancel_backup', { taskId }),
  restoreBackup: (backupPath, destination) => invoke('restore_backup', { backupPath, destination }),
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

  notify: async (title, body) => {
    try {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === 'granted';
      if (granted) sendNotification({ title, body });
    } catch {}
  },

  onBackupStarted: (cb) => listen('backup-started', (e) => cb(e.payload)),
  onBackupProgress: (cb) => listen('backup-progress', (e) => cb(e.payload)),
  onBackupComplete: (cb) => listen('backup-complete', (e) => cb(e.payload)),
  onRestoreProgress: (cb) => listen('restore-progress', (e) => cb(e.payload)),
  onTaskUpdated: (cb) => listen('task-updated', (e) => cb(e.payload)),
};
