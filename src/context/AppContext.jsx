import React, { createContext, useContext, useState, useEffect, useCallback, useRef } from 'react';
import { v4 as uuidv4 } from 'uuid';
import { bridge } from '../lib/tauri';
import { useSystemTheme } from '../hooks/useSystemTheme';
import { DEFAULT_ACCENT } from '../lib/accent';
import { DEFAULT_LANGUAGE, SUPPORTED_LANGUAGES, translate } from '../lib/i18n';

const AppContext = createContext(null);

const DEFAULT_SETTINGS = {
  defaultDestination: '',
  excludePatterns: '',
  confirmBeforeBackup: true,
  showNotifications: true,
  accentColor: DEFAULT_ACCENT,
  theme: 'system',
  language: DEFAULT_LANGUAGE,
  verify: false,
  continueOnError: true,
  preserveMtime: true,
  sidebarOpen: true,
  lastView: 'home',
};

export function AppProvider({ children }) {
  const [tasks, setTasks] = useState([]);
  const [history, setHistory] = useState([]);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [activeBackups, setActiveBackups] = useState({});
  const [loaded, setLoaded] = useState(false);
  const [toast, setToast] = useState(null);
  const [confirmState, setConfirmState] = useState(null);
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  // Local helper bound to the current locale. The provider can't use the
  // useT() hook because it *is* the provider, so it goes through translate()
  // directly. Reads from the ref so async callbacks (event listeners) pick
  // up the active language without re-binding.
  const tr = useCallback((key, params) => {
    const lang = SUPPORTED_LANGUAGES.includes(settingsRef.current.language)
      ? settingsRef.current.language
      : DEFAULT_LANGUAGE;
    return translate(lang, key, params);
  }, []);

  const { resolved: resolvedTheme } = useSystemTheme(settings.theme);

  useEffect(() => {
    (async () => {
      try {
        const [s, t, h] = await Promise.all([
          bridge.getSettings(),
          bridge.getTasks(),
          bridge.getHistory(),
        ]);
        setSettings({ ...DEFAULT_SETTINGS, ...(s || {}) });
        setTasks(Array.isArray(t) ? t : []);
        setHistory(Array.isArray(h) ? h : []);
      } finally {
        setLoaded(true);
      }
    })();
  }, []);

  // Tasks are persisted on user action (addTask / editTask / deleteTask).
  // Rust owns lastBackup via task-updated events. Removing the auto-save
  // useEffect avoids clobbering writes from the background scheduler.

  useEffect(() => {
    if (!loaded) return;
    bridge.saveHistory(history);
  }, [history, loaded]);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolvedTheme);
    // Accent is fixed to the default — picker was removed from Settings.
    document.documentElement.setAttribute('data-accent', DEFAULT_ACCENT);
  }, [resolvedTheme]);

  useEffect(() => {
    const unlisten = [];
    let cancelled = false;
    (async () => {
      const offStart = await bridge.onBackupStarted((data) => {
        setActiveBackups((prev) => ({
          ...prev,
          [data.taskId]: { progress: 0, copiedBytes: 0, totalBytes: 0, copiedFiles: 0, totalFiles: 0 },
        }));
      });
      const offProgress = await bridge.onBackupProgress((data) => {
        setActiveBackups((prev) => ({ ...prev, [data.taskId]: data }));
      });
      const offComplete = await bridge.onBackupComplete((data) => {
        setActiveBackups((prev) => {
          const next = { ...prev };
          delete next[data.taskId];
          return next;
        });
        // lastBackup is now owned by Rust — it emits task-updated which the
        // listener below applies. Don't mutate tasks locally on complete.
        const existingTask = tasksRef.current.find((t) => t.id === data.taskId);
        setHistory((prev) => [
          {
            id: uuidv4(),
            taskId: data.taskId,
            taskName: existingTask?.name || 'Backup',
            timestamp: new Date().toISOString(),
            status: data.success ? 'success' : data.cancelled ? 'cancelled' : 'error',
            path: data.path,
            totalBytes: data.totalBytes,
            totalFiles: data.totalFiles,
            durationMs: data.durationMs,
            error: data.error,
            skipped: data.skipped,
            unchanged: data.unchanged,
            failed: data.failed,
            verified: data.verified,
            unreadable: data.unreadable,
          },
          ...prev,
        ]);
        if (data.success) {
          showToast(tr('backup.toast.complete'));
          if (settingsRef.current.showNotifications) {
            bridge.notify(
              tr('backup.notification.title'),
              tr('backup.notification.body', { name: existingTask?.name || tr('view.tasks').toLowerCase() }),
            );
          }
        } else if (data.cancelled) {
          showToast(tr('backup.toast.cancelled'));
        } else {
          showToast(tr('backup.toast.failed', { error: data.error }), 'error');
        }
      });
      const offTaskUpdated = await bridge.onTaskUpdated((data) => {
        if (!data || !data.id) return;
        setTasks((prev) => {
          const next = prev.map((t) => (t.id === data.id ? { ...t, ...data } : t));
          // Don't re-persist — Rust already wrote tasks.json.
          return next;
        });
      });
      if (cancelled) {
        offStart?.(); offProgress?.(); offComplete?.(); offTaskUpdated?.();
      } else {
        unlisten.push(offStart, offProgress, offComplete, offTaskUpdated);
      }
    })();
    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn?.());
    };
  }, []);

  const showToast = useCallback((message, kind = 'info') => {
    setToast({ message, kind, id: Date.now() });
    setTimeout(() => setToast(null), 3000);
  }, []);

  const confirm = useCallback((opts) => new Promise((resolve) => {
    // If a previous confirm dialog is still open, settle its promise
    // before replacing it — otherwise the awaiter hangs forever (#15).
    setConfirmState((prev) => {
      prev?.resolve(false);
      return { ...opts, resolve };
    });
  }), []);

  const handleConfirm = useCallback((value) => {
    setConfirmState((prev) => {
      prev?.resolve(value);
      return null;
    });
  }, []);

  const startBackup = useCallback(async (task) => {
    if (settings.confirmBeforeBackup) {
      const ok = await confirm({
        title: tr('task.confirm.backup.title', { name: task.name }),
        body: tr('task.confirm.backup.body', { source: task.source, destination: task.destination }),
        confirmLabel: tr('task.confirm.backup.action'),
      });
      if (!ok) return;
    }
    try {
      await bridge.startBackup(task, settings);
    } catch (e) {
      showToast(tr('backup.toast.failed', { error: e }), 'error');
    }
  }, [settings, confirm, showToast, tr]);

  const cancelBackup = useCallback(async (taskId) => {
    await bridge.cancelBackup(taskId);
  }, []);

  const addTask = useCallback((taskDraft) => {
    const dest = taskDraft.destination || settings.defaultDestination;
    if (!taskDraft.name || !taskDraft.source || !dest) return false;
    setTasks((prev) => {
      const next = [
        ...prev,
        { id: uuidv4(), ...taskDraft, destination: dest, lastBackup: null },
      ];
      bridge.saveTasks(next);
      return next;
    });
    return true;
  }, [settings.defaultDestination]);

  const editTask = useCallback((id, patch) => {
    setTasks((prev) => {
      const next = prev.map((t) => (t.id === id ? { ...t, ...patch } : t));
      bridge.saveTasks(next);
      return next;
    });
  }, []);

  const deleteTask = useCallback(async (id) => {
    const ok = await confirm({
      title: tr('task.confirm.delete.title'),
      body: tr('task.confirm.delete.body'),
      confirmLabel: tr('common.delete'),
      danger: true,
    });
    if (!ok) return;
    setTasks((prev) => {
      const next = prev.filter((t) => t.id !== id);
      bridge.saveTasks(next);
      return next;
    });
  }, [confirm, tr]);

  const deleteHistory = useCallback((id) => {
    setHistory((prev) => prev.filter((h) => h.id !== id));
  }, []);

  const clearHistory = useCallback(async () => {
    const ok = await confirm({
      title: tr('history.confirm.clear.title'),
      body: tr('history.confirm.clear.body'),
      confirmLabel: tr('history.confirm.clear.action'),
      danger: true,
    });
    if (!ok) return;
    setHistory([]);
  }, [confirm, tr]);

  const updateSetting = useCallback((key, value) => {
    setSettings((prev) => {
      const next = { ...prev, [key]: value };
      bridge.saveSettings(next);
      return next;
    });
  }, []);

  const revealFolder = useCallback(async (folderPath) => {
    const res = await bridge.revealFolder(folderPath);
    if (!res.success) showToast(tr('reveal.cannot_open', { error: res.error }), 'error');
  }, [showToast, tr]);

  const restoreBackup = useCallback(async (backupPath) => {
    const destination = await bridge.selectDirectory(tr('restore.dialog.select'));
    if (!destination) return;
    const ok = await confirm({
      title: tr('restore.dialog.title'),
      body: tr('restore.dialog.body', { destination }),
      confirmLabel: tr('restore.dialog.action'),
    });
    if (!ok) return;
    try {
      const res = await bridge.restoreBackup(backupPath, destination);
      if (res.success) {
        showToast(tr('restore.toast.success', { n: res.copiedFiles }));
      } else {
        showToast(tr('restore.toast.failed', { error: res.error }), 'error');
      }
    } catch (e) {
      showToast(tr('restore.toast.failed', { error: e }), 'error');
    }
  }, [confirm, showToast, tr]);

  const value = {
    tasks, history, settings, activeBackups, loaded, toast, confirmState, resolvedTheme,
    startBackup, cancelBackup, addTask, editTask, deleteTask,
    deleteHistory, clearHistory, updateSetting, revealFolder, restoreBackup,
    showToast, handleConfirm, confirm, tr,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp() {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error('useApp must be used within AppProvider');
  return ctx;
}
