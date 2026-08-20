import React, { createContext, useContext, useState, useEffect, useCallback, useMemo } from 'react';
import { bridge } from '../lib/tauri';

const ProgressContext = createContext(null);

// The 10Hz state lives here, apart from AppContext, so a running backup
// re-renders only the two components that show progress instead of the
// whole tree. This provider sits *outside* AppProvider and depends on
// nothing — AppContext consumes it, not the other way round.
export function ProgressProvider({ children }) {
  const [activeBackups, setActiveBackups] = useState({});
  const [activeRestore, setActiveRestore] = useState(null);

  useEffect(() => {
    const unlisten = [];
    let cancelled = false;
    (async () => {
      const offStart = await bridge.onBackupStarted((data) => {
        setActiveBackups((prev) => ({
          ...prev,
          [data.taskId]: {
            progress: 0,
            copiedBytes: 0,
            totalBytes: 0,
            copiedFiles: 0,
            totalFiles: 0,
            phase: 'copying',
          },
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
      });
      const offRestore = await bridge.onRestoreProgress((data) => {
        // Only meaningful while a restore is in flight; beginRestore seeds
        // the state and endRestore clears it, so a late straggler event
        // after completion can't resurrect the overlay.
        setActiveRestore((prev) => (prev ? { ...prev, ...data, indeterminate: false } : prev));
      });
      if (cancelled) {
        offStart?.(); offProgress?.(); offComplete?.(); offRestore?.();
      } else {
        unlisten.push(offStart, offProgress, offComplete, offRestore);
      }
    })();
    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn?.());
    };
  }, []);

  const beginRestore = useCallback(() => {
    setActiveRestore({
      indeterminate: true,
      progress: 0,
      copiedFiles: 0,
      totalFiles: 0,
      copiedBytes: 0,
      totalBytes: 0,
    });
  }, []);

  const endRestore = useCallback(() => setActiveRestore(null), []);

  const value = useMemo(
    () => ({ activeBackups, activeRestore, beginRestore, endRestore }),
    [activeBackups, activeRestore, beginRestore, endRestore],
  );

  return <ProgressContext.Provider value={value}>{children}</ProgressContext.Provider>;
}

export function useProgress() {
  const ctx = useContext(ProgressContext);
  if (!ctx) throw new Error('useProgress must be used within ProgressProvider');
  return ctx;
}
