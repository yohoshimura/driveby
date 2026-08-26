import React, { useEffect, useRef } from 'react';
import Button from './common/Button';
import { useExitTransition } from '../hooks/useExitTransition';
import { useFormat } from '../hooks/useFormat';
import { useT } from '../hooks/useT';

/// The confirmation that replaces "From: … To: …" with what the run would
/// actually do. It opens *before* the answer is known — the scan is a full
/// source walk plus a stat of every counterpart, which on a large tree over
/// USB is not instant — so it has a waiting state of its own, and cancelling
/// from it stops the scan rather than just closing a window.
export default function BackupPreviewDialog({ state, onConfirm, onCancel }) {
  const t = useT();
  const { formatBytes, formatNumber } = useFormat();
  const dialogRef = useRef(null);
  const lastStateRef = useRef(state);
  if (state) lastStateRef.current = state;

  const { mounted, state: anim } = useExitTransition(!!state, 200);

  useEffect(() => {
    if (!state) return undefined;
    dialogRef.current?.focus();
    // Capture phase, like ConfirmDialog: two bubble-phase listeners on the
    // same target cannot suppress each other, which is how Escape used to
    // close the task form behind the dialog as well (#F5).
    const handler = (e) => {
      if (e.key !== 'Escape' || e.repeat) return;
      e.stopPropagation();
      onCancel();
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [state, onCancel]);

  if (!mounted) return null;
  const visible = state || lastStateRef.current;
  if (!visible) return null;

  const { task, status, payload } = visible;
  const destinations = payload?.destinations || [];
  const totals = destinations.reduce(
    (acc, d) => ({
      changes: acc.changes + d.newFiles + d.modifiedFiles + d.deletedFiles,
      deletions: acc.deletions + d.deletedFiles,
    }),
    { changes: 0, deletions: 0 },
  );
  const ready = status === 'ready';
  const nothingToDo = ready && totals.changes === 0;

  return (
    <div
      className="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="preview-title"
      aria-busy={!ready}
      data-state={anim}
    >
      <div className="modal__backdrop" onClick={onCancel} />
      <div className="modal__content modal__content--wide" ref={dialogRef} tabIndex={-1}>
        <h2 id="preview-title" className="modal__title">
          {t('task.confirm.backup.title', { name: task?.name })}
        </h2>

        {!ready ? (
          <p className="modal__body">{t('preview.scanning')}</p>
        ) : (
          <div className="preview">
            {nothingToDo && <p className="modal__body">{t('preview.nothing')}</p>}
            {destinations.map((d) => (
              <div className="preview-dest" key={d.path}>
                <div className="preview-dest__path" title={d.path}>{d.path}</div>
                {d.reachable ? (
                  <div className="preview-dest__stats">
                    <span className="preview-stat">
                      <b>{formatNumber(d.newFiles)}</b> {t('preview.label.new')}
                      <em>{formatBytes(d.newBytes)}</em>
                    </span>
                    <span className="preview-stat">
                      <b>{formatNumber(d.modifiedFiles)}</b> {t('preview.label.modified')}
                      <em>{formatBytes(d.modifiedBytes)}</em>
                    </span>
                    {/* The only destructive number in the dialog, and the
                        reason the dialog exists: coloured when it is not
                        zero, plain when it is. */}
                    <span className={`preview-stat ${d.deletedFiles > 0 ? 'preview-stat--danger' : ''}`}>
                      <b>{formatNumber(d.deletedFiles)}</b> {t('preview.label.deleted')}
                      <em>{formatBytes(d.deletedBytes)}</em>
                    </span>
                    <span className="preview-stat preview-stat--muted">
                      <b>{formatNumber(d.unchangedFiles)}</b> {t('preview.label.unchanged')}
                    </span>
                  </div>
                ) : (
                  <div className="preview-dest__absent">{t('history.dest.unreachable')}</div>
                )}
              </div>
            ))}
            {payload?.unreadable > 0 && (
              <div className="preview-note">
                {t('history.unreadable', { n: payload.unreadable, count: payload.unreadable })}
              </div>
            )}
          </div>
        )}

        <div className="modal__actions">
          <Button onClick={onCancel}>{t('common.cancel')}</Button>
          <Button
            variant="primary"
            destructive={totals.deletions > 0}
            onClick={onConfirm}
            disabled={!ready}
          >
            {nothingToDo ? t('preview.action.anyway') : t('task.confirm.backup.action')}
          </Button>
        </div>
      </div>
    </div>
  );
}
