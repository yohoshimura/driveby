import React, {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useMemo,
  useRef,
} from 'react';
import { v4 as uuidv4 } from 'uuid';
import { bridge } from '../lib/tauri';
import { useSystemTheme } from '../hooks/useSystemTheme';
import { useProgress } from './ProgressContext';
import { DEFAULT_ACCENT } from '../lib/accent';
import { DEFAULT_HISTORY_RETENTION, trimHistory } from '../lib/history';
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
  parallelCopies: 4,
  historyRetention: DEFAULT_HISTORY_RETENTION,
  sidebarOpen: true,
  lastView: 'home',
};

export function AppProvider({ children }) {
  const [tasks, setTasks] = useState([]);
  const [history, setHistory] = useState([]);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [loaded, setLoaded] = useState(false);
  const [toast, setToast] = useState(null);
  const [confirmState, setConfirmState] = useState(null);
  const { beginRestore, endRestore } = useProgress();
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  // Armed by the user-facing mutators only, so the persist effects below
  // stay inert for state that Rust pushed at us (task-updated). Persisting
  // from an effect rather than from inside a setState updater keeps the
  // updaters pure — StrictMode invokes them twice in development, which
  // used to mint two uuids and fire two conflicting saves (#F6).
  const tasksDirty = useRef(false);
  const settingsDirty = useRef(false);
  const toastTimer = useRef(null);
  // A restore is a single global operation; the ref is the in-flight guard
  // that stops a double-click launching a second one before the backend's
  // own RestoreState can refuse it.
  const restoreBusy = useRef(false);

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
        // historyLimit was a *count*; historyRetention is an *age*. There is
        // no meaningful mapping between them, so the old key is dropped
        // rather than left behind in settings.json for nobody to read.
        const { historyLimit: _dropped, ...stored } = s || {};
        const merged = { ...DEFAULT_SETTINGS, ...stored };
        setSettings(merged);
        setTasks(Array.isArray(t) ? t : []);
        // Trim at load too, so a history.json written before the window
        // existed — or under a longer one — shrinks on the next save.
        setHistory(trimHistory(h, merged.historyRetention));
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
    if (!loaded || !tasksDirty.current) return;
    tasksDirty.current = false;
    bridge.saveTasks(tasks);
  }, [tasks, loaded]);

  useEffect(() => {
    if (!loaded || !settingsDirty.current) return;
    settingsDirty.current = false;
    bridge.saveSettings(settings);
  }, [settings, loaded]);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolvedTheme);
    // Accent is fixed to the default — picker was removed from Settings.
    document.documentElement.setAttribute('data-accent', DEFAULT_ACCENT);
  }, [resolvedTheme]);

  useEffect(() => {
    const unlisten = [];
    let cancelled = false;
    (async () => {
      // Progress events are ProgressContext's business; this listener is
      // here for the history row and the toast, which AppContext owns.
      const offComplete = await bridge.onBackupComplete((data) => {
        // lastBackup is now owned by Rust — it emits task-updated which the
        // listener below applies. Don't mutate tasks locally on complete.
        const existingTask = tasksRef.current.find((t) => t.id === data.taskId);
        setHistory((prev) =>
          trimHistory(
            [
              {
                id: uuidv4(),
                taskId: data.taskId,
                taskName: existingTask?.name || tr('common.backup'),
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
            ],
            settingsRef.current.historyRetention,
          ),
        );
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
        offComplete?.(); offTaskUpdated?.();
      } else {
        unlisten.push(offComplete, offTaskUpdated);
      }
    })();
    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn?.());
    };
  }, []);

  // Clear the pending timer before arming a new one: two toasts in quick
  // succession used to share the first one's deadline, so the second
  // vanished early.
  const showToast = useCallback((message, kind = 'info') => {
    setToast({ message, kind, id: Date.now() });
    if (toastTimer.current) clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => {
      toastTimer.current = null;
      setToast(null);
    }, 3000);
  }, []);

  useEffect(() => () => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
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
    // Built outside the updater: StrictMode calls the updater twice, and
    // minting the id inside it produced two different tasks.
    const created = { id: uuidv4(), ...taskDraft, destination: dest, lastBackup: null };
    tasksDirty.current = true;
    setTasks((prev) => [...prev, created]);
    return true;
  }, [settings.defaultDestination]);

  const editTask = useCallback((id, patch) => {
    tasksDirty.current = true;
    setTasks((prev) => prev.map((t) => (t.id === id ? { ...t, ...patch } : t)));
  }, []);

  const deleteTask = useCallback(async (id) => {
    const ok = await confirm({
      title: tr('task.confirm.delete.title'),
      body: tr('task.confirm.delete.body'),
      confirmLabel: tr('common.delete'),
      danger: true,
    });
    if (!ok) return;
    tasksDirty.current = true;
    setTasks((prev) => prev.filter((t) => t.id !== id));
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
    settingsDirty.current = true;
    setSettings((prev) => ({ ...prev, [key]: value }));
    // Shortening the window has to bite now, not at the next completed run —
    // otherwise the setting looks like it did nothing.
    if (key === 'historyRetention') {
      setHistory((prev) => trimHistory(prev, value));
    }
  }, []);

  const revealFolder = useCallback(async (folderPath) => {
    const res = await bridge.revealFolder(folderPath);
    if (!res.success) showToast(tr('reveal.cannot_open', { error: res.error }), 'error');
  }, [showToast, tr]);

  const restoreBackup = useCallback(async (backupPath) => {
    if (restoreBusy.current) {
      showToast(tr('restore.busy'), 'error');
      return;
    }
    const destination = await bridge.selectDirectory(tr('restore.dialog.select'));
    if (!destination) return;
    const ok = await confirm({
      title: tr('restore.dialog.title'),
      // Naming only the destination made it impossible to notice that a
      // shifted History row had selected a different backup (#F7).
      body: tr('restore.dialog.body', { source: backupPath, destination }),
      confirmLabel: tr('restore.dialog.action'),
    });
    if (!ok) return;
    restoreBusy.current = true;
    beginRestore();
    try {
      const res = await bridge.restoreBackup(backupPath, destination);
      if (res.cancelled) {
        showToast(tr('restore.toast.cancelled'));
      } else if (res.success) {
        showToast(tr('restore.toast.success', { n: res.copiedFiles, count: res.copiedFiles }));
      } else {
        showToast(tr('restore.toast.failed', { error: res.error }), 'error');
      }
    } catch (e) {
      showToast(tr('restore.toast.failed', { error: e }), 'error');
    } finally {
      restoreBusy.current = false;
      endRestore();
    }
  }, [confirm, showToast, tr, beginRestore, endRestore]);

  const cancelRestore = useCallback(async () => {
    await bridge.cancelRestore();
  }, []);

  // Memoised so a value identity change — and the re-render of every
  // consumer that comes with it — happens only when something actually
  // changed, not on each render of the provider.
  // Deliberately no activeRestore/activeBackups here — live progress stays
  // in ProgressContext so a running job doesn't invalidate this value ten
  // times a second and re-render every consumer with it.
  const value = useMemo(() => ({
    tasks, history, settings, loaded, toast, confirmState, resolvedTheme,
    startBackup, cancelBackup, addTask, editTask, deleteTask,
    deleteHistory, clearHistory, updateSetting, revealFolder, restoreBackup, cancelRestore,
    showToast, handleConfirm, confirm, tr,
  }), [
    tasks, history, settings, loaded, toast, confirmState, resolvedTheme,
    startBackup, cancelBackup, addTask, editTask, deleteTask,
    deleteHistory, clearHistory, updateSetting, revealFolder, restoreBackup, cancelRestore,
    showToast, handleConfirm, confirm, tr,
  ]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp() {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error('useApp must be used within AppProvider');
  return ctx;
}
