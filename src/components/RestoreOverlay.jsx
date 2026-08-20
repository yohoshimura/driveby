import React from 'react';
import Button from './common/Button';
import { useApp } from '../context/AppContext';
import { useProgress } from '../context/ProgressContext';
import { useFormat } from '../hooks/useFormat';
import { useT } from '../hooks/useT';

// A restore used to run with no feedback at all: the window sat idle for
// the whole copy and the only way out was to wait. This is the surface for
// both halves of that fix — progress, and the Stop button.
export default function RestoreOverlay() {
  const { activeRestore } = useProgress();
  const { cancelRestore } = useApp();
  const t = useT();
  const { formatBytes, formatNumber } = useFormat();

  if (!activeRestore) return null;

  const { indeterminate, progress = 0, copiedFiles = 0, totalFiles = 0, copiedBytes = 0, totalBytes = 0 } =
    activeRestore;

  return (
    <div className="restore-overlay" role="status" aria-live="polite">
      <div className="restore-overlay__body">
        <div className="restore-overlay__title">{t('restore.progress.title')}</div>
        <div
          className={`progressbar ${indeterminate ? 'progressbar--indeterminate' : ''}`}
          role="progressbar"
          aria-valuenow={indeterminate ? undefined : progress}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className="progressbar__fill"
            style={{ width: indeterminate ? '100%' : `${progress}%` }}
          />
        </div>
        <div className="restore-overlay__meta">
          {indeterminate ? (
            <span>{t('restore.progress.starting')}</span>
          ) : (
            <>
              <span className="mono">
                {formatNumber(copiedFiles)}/{formatNumber(totalFiles)}
              </span>
              <span className="mono">
                {formatBytes(copiedBytes)} / {formatBytes(totalBytes)}
              </span>
            </>
          )}
        </div>
      </div>
      <Button size="small" destructive onClick={cancelRestore} ariaLabel={t('restore.action.stop')}>
        {t('restore.action.stop')}
      </Button>
    </div>
  );
}
