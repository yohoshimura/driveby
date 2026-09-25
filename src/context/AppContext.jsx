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
import { DEFAULT_UI_STYLE, platformOf, resolveUiStyle } from '../lib/uiStyle';
import { DEFAULT_HISTORY_RETENTION, trimHistory } from '../lib/history';
import { migrateTasks, taskDestinations, taskSources } from '../lib/task';
import { DEFAULT_LANGUAGE, SUPPORTED_LANGUAGES, translate } from '../lib/i18n';
import { makeFormatters } from '../lib/format';

const AppContext = createContext(null);

const DEFAULT_SETTINGS = {
  defaultDestination: '',
  excludePatterns: '',
  confirmBeforeBackup: true,
  showNotifications: true,
  accentColor: DEFAULT_ACCENT,
  theme: 'system',
  uiStyle: DEFAULT_UI_STYLE,
  language: DEFAULT_LANGUAGE,
  verify: false,
  continueOnError: true,
  preserveMtime: true,
  parallelCopies: 4,
  maxSpeedMbps: 0,
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
  // { task, status: 'scanning' | 'ready', payload } while the pre-run
  // preview dialog is open, null when it is not.
  const [previewState, setPreviewState] = useState(null);
  // { taskId: [absent destination paths] }, pushed by the scheduler.
  const [destinationStatus, setDestinationStatus] = useState({});
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
  const currentLanguage = useCallback(() => (
    SUPPORTED_LANGUAGES.includes(settingsRef.current.language)
      ? settingsRef.current.language
      : DEFAULT_LANGUAGE
  ), []);
  const tr = useCallback(
    (key, params) => translate(currentLanguage(), key, params),
    [currentLanguage],
  );

  const { resolved: resolvedTheme } = useSystemTheme(settings.theme);
  const resolvedStyle = resolveUiStyle(settings.uiStyle, navigator.userAgent);

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
        // A tasks.json written before 1.7.2 holds one `destination` per
        // task, and one written before sources had folders one `source`.
        // Rewrite it into the plural shapes on first load — and only then:
        // migrateTasks hands back the same array when there was nothing to
        // do, so an ordinary launch doesn't rewrite the file.
        const storedTasks = Array.isArray(t) ? t : [];
        const live = migrateTasks(storedTasks);
        if (live !== storedTasks) tasksDirty.current = true;
        setTasks(live);
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

  // Separate from data-theme on purpose: the style (iOS, Windows 11, GNOME)
  // and the light/dark mode are two independent axes, and each stylesheet in
  // src/themes/ keys its dark variant off both attributes together.
  useEffect(() => {
    document.documentElement.setAttribute('data-style', resolvedStyle);
  }, [resolvedStyle]);

  // The OS rather than the style, for the few rules that answer to what is
  // installed there — the font a style falls back to when its own is absent.
  useEffect(() => {
    document.documentElement.setAttribute('data-platform', platformOf(navigator.userAgent));
  }, []);

  // The desktop's accent colour, which the Adwaita style paints with as
  // libadwaita apps do (src/themes/gnome.css). Set on the root for every
  // style but read by that one alone; without it, Adwaita keeps its blue.
  useEffect(() => {
    const root = document.documentElement.style;
    const apply = (hex) => {
      if (hex) root.setProperty('--system-accent', hex);
      else root.removeProperty('--system-accent');
    };
    let off;
    let cancelled = false;
    bridge.systemAccent().then(apply).catch(() => {});
    bridge.onSystemAccent(apply).then((fn) => {
      if (cancelled) fn();
      else off = fn;
    });
    return () => {
      cancelled = true;
      off?.();
    };
  }, []);

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
        // Named up front so the notification's "View in History" can point
        // at the row it is about to describe.
        const historyId = uuidv4();
        const viewHistory = { kind: 'viewHistory', label: tr('backup.notification.view_history'), historyId };
        setHistory((prev) =>
          trimHistory(
            [
              {
                id: historyId,
                taskId: data.taskId,
                taskName: existingTask?.name || tr('common.backup'),
                timestamp: new Date().toISOString(),
                status: data.success
                  ? 'success'
                  : data.cancelled
                    ? 'cancelled'
                    : data.partial
                      ? 'partial'
                      : 'error',
                path: data.path,
                destinations: data.destinations,
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
            const name = existingTask?.name || tr('view.tasks').toLowerCase();
            // The task keeps daily versions, and this drive cannot. The backup
            // is fine, but the user is counting on something that is not there,
            // and a scheduled run has no other way to say so.
            const noVersions = (data.destinations || []).find((d) => d.versionsUnavailable);
            bridge.notify(
              tr('backup.notification.title'),
              noVersions
                ? tr('backup.notification.body_no_versions', { name, path: noVersions.path })
                : tr('backup.notification.body', { name }),
              [
                // `path` is the first destination written; with several,
                // the others are one click away in the History row.
                ...(data.path
                  ? [{ kind: 'openFolder', label: tr('backup.notification.open_folder'), path: data.path }]
                  : []),
                viewHistory,
              ],
            );
          }
        } else if (data.cancelled) {
          showToast(tr('backup.toast.cancelled'));
        } else if (data.partial) {
          // Some destinations were written and some were not. Said plainly,
          // with the count, because "failed" would be wrong and "complete"
          // would be a lie.
          const all = data.destinations || [];
          showToast(
            tr('backup.toast.partial', {
              done: all.filter((d) => d.status === 'success').length,
              total: all.length,
            }),
            'error',
          );
        } else {
          // A destination refused for room is said in so many words, with
          // the figures in the reader's language, and notified as well: a
          // scheduled run is refused with the window closed, and nothing else
          // would tell the user their backups have stopped.
          const short = (data.destinations || []).find((d) => d.status === 'nospace');
          if (short) {
            const { formatBytes } = makeFormatters(currentLanguage());
            const message = tr('backup.nospace', {
              path: short.path,
              needed: formatBytes(short.neededBytes),
              free: formatBytes(short.availableBytes),
            });
            showToast(message, 'error');
            if (settingsRef.current.showNotifications) {
              bridge.notify(tr('backup.notification.title'), message, [viewHistory]);
            }
          } else {
            showToast(tr('backup.toast.failed', { error: data.error }), 'error');
          }
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
      // The scheduler stats every destination each minute and only speaks
      // up when the answer changes, so each payload is the whole picture
      // and replaces what we hold. A task dropping out of the list is how
      // we learn its drive is back.
      const offDestinations = await bridge.onDestinationsStatus((list) => {
        const next = {};
        for (const entry of list || []) next[entry.taskId] = entry.missing;
        setDestinationStatus(next);
      });
      // Fired once per absence, not once per missed occurrence: the drive
      // in the drawer must not become a daily notification.
      const offMissing = await bridge.onDestinationMissing((data) => {
        const message = tr('backup.toast.destination_missing', { name: data?.taskName });
        showToast(message, 'error');
        if (settingsRef.current.showNotifications) {
          bridge.notify(tr('backup.notification.title'), message);
        }
      });
      if (cancelled) {
        offComplete?.(); offTaskUpdated?.(); offDestinations?.(); offMissing?.();
      } else {
        unlisten.push(offComplete, offTaskUpdated, offDestinations, offMissing);
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

  const launchBackup = useCallback(async (task) => {
    try {
      await bridge.startBackup(task, settings);
    } catch (e) {
      showToast(tr('backup.toast.failed', { error: e }), 'error');
    }
  }, [settings, showToast, tr]);

  const startBackup = useCallback(async (task) => {
    if (!settings.confirmBeforeBackup) {
      await launchBackup(task);
      return;
    }
    // The dialog opens *before* the answer is known. Working out what would
    // change is a full source walk plus a stat of every counterpart, which
    // on a large tree over USB takes long enough that a button which
    // appeared to do nothing would be pressed again.
    setPreviewState({ task, status: 'scanning', payload: null });
    try {
      const payload = await bridge.previewBackup(task, settings);
      // Cancelling closed the dialog already; a late answer must not
      // reopen it.
      if (payload.cancelled) return;
      setPreviewState((prev) => (
        prev && prev.task.id === task.id ? { ...prev, status: 'ready', payload } : prev
      ));
    } catch (e) {
      setPreviewState(null);
      showToast(tr('backup.toast.failed', { error: e }), 'error');
    }
  }, [settings, launchBackup, showToast, tr]);

  // Read outside the updater and launched after it: StrictMode invokes
  // updaters twice, and a side effect inside one starts two backups (#F6).
  const confirmPreview = useCallback(() => {
    const pending = previewState?.task;
    setPreviewState(null);
    if (pending) launchBackup(pending);
  }, [previewState, launchBackup]);

  const cancelPreview = useCallback(() => {
    setPreviewState(null);
    // Stop the scan too, not just the window: it is holding a disk busy.
    bridge.cancelPreview();
  }, []);

  const cancelBackup = useCallback(async (taskId) => {
    await bridge.cancelBackup(taskId);
  }, []);

  const addTask = useCallback((taskDraft) => {
    const listed = taskDestinations(taskDraft);
    const destinations = listed.length > 0
      ? listed
      : taskDestinations({ destination: settings.defaultDestination });
    if (!taskDraft.name || taskSources(taskDraft).length === 0 || destinations.length === 0) return false;
    // Built outside the updater: StrictMode calls the updater twice, and
    // minting the id inside it produced two different tasks.
    const created = { id: uuidv4(), ...taskDraft, destinations, lastBackup: null };
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

  const restoreBackup = useCallback(async (backupPath, preferredDay = null) => {
    if (restoreBusy.current) {
      showToast(tr('restore.busy'), 'error');
      return;
    }
    // A destination with daily versions holds one whole backup per day, and
    // the backend refuses to restore its root. Ask which day, preselecting the
    // one this History row wrote while it is still kept.
    let source = backupPath;
    const days = await bridge.listSnapshots(backupPath).catch(() => []);
    if (days.length > 0) {
      const { formatDay } = makeFormatters(currentLanguage());
      const picked = await confirm({
        title: tr('restore.day.title'),
        body: tr('restore.day.body'),
        choices: days.map((d) => ({ value: d.path, label: formatDay(d.name) })),
        initialChoice: (days.find((d) => d.name === preferredDay) || days[0]).path,
        confirmLabel: tr('restore.day.action'),
      });
      if (!picked) return;
      source = picked;
    }
    const destination = await bridge.selectDirectory(tr('restore.dialog.select'));
    if (!destination) return;
    const ok = await confirm({
      title: tr('restore.dialog.title'),
      // Naming only the destination made it impossible to notice that a
      // shifted History row had selected a different backup (#F7).
      body: tr('restore.dialog.body', { source, destination }),
      confirmLabel: tr('restore.dialog.action'),
    });
    if (!ok) return;
    restoreBusy.current = true;
    beginRestore();
    try {
      const res = await bridge.restoreBackup(source, destination);
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
    tasks, history, settings, loaded, toast, confirmState, resolvedTheme, previewState,
    destinationStatus,
    startBackup, cancelBackup, addTask, editTask, deleteTask,
    deleteHistory, clearHistory, updateSetting, revealFolder, restoreBackup, cancelRestore,
    confirmPreview, cancelPreview,
    showToast, handleConfirm, confirm, tr,
  }), [
    tasks, history, settings, loaded, toast, confirmState, resolvedTheme, previewState,
    destinationStatus,
    startBackup, cancelBackup, addTask, editTask, deleteTask,
    deleteHistory, clearHistory, updateSetting, revealFolder, restoreBackup, cancelRestore,
    confirmPreview, cancelPreview,
    showToast, handleConfirm, confirm, tr,
  ]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp() {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error('useApp must be used within AppProvider');
  return ctx;
}
