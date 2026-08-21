import React, { useEffect, useRef } from 'react';
import Button from './common/Button';
import { useExitTransition } from '../hooks/useExitTransition';
import { useT } from '../hooks/useT';

export default function ConfirmDialog({ state, onResolve }) {
  const t = useT();
  const dialogRef = useRef(null);
  const lastStateRef = useRef(state);
  if (state) lastStateRef.current = state;

  const { mounted, state: anim } = useExitTransition(!!state, 200);

  useEffect(() => {
    if (!state) return;
    dialogRef.current?.focus();
    // Capture phase, so this runs before the app-wide shortcuts that also
    // listen on window. Two bubble-phase listeners on the same target can't
    // suppress each other, which is why cancelling a confirmation used to
    // close the task form behind it and throw away what was typed (#F5).
    const openedAt = Date.now();
    const handler = (e) => {
      if (e.key !== 'Escape' && e.key !== 'Enter') return;
      // Holding or double-tapping the key that opened this dialog fires a
      // second keydown ~500ms later. Confirming on that repeat destroyed
      // things before the dialog had been read (#F1).
      if (e.repeat) return;
      if (e.key === 'Escape') {
        e.stopPropagation();
        onResolve(false);
        return;
      }
      if (Date.now() - openedAt < 250) return;
      if (!dialogRef.current?.contains(document.activeElement)) return;
      e.stopPropagation();
      onResolve(true);
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [state, onResolve]);

  if (!mounted) return null;
  const visible = state || lastStateRef.current;
  if (!visible) return null;

  return (
    <div
      className="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-title"
      data-state={anim}
    >
      <div className="modal__backdrop" onClick={() => onResolve(false)} />
      <div className="modal__content" ref={dialogRef} tabIndex={-1}>
        <h2 id="confirm-title" className="modal__title">{visible.title}</h2>
        <p className="modal__body">{visible.body}</p>
        <div className="modal__actions">
          <Button onClick={() => onResolve(false)}>{t('common.cancel')}</Button>
          <Button
            variant="primary"
            destructive={visible.danger}
            onClick={() => onResolve(true)}
          >
            {visible.confirmLabel || t('common.ok')}
          </Button>
        </div>
      </div>
    </div>
  );
}
