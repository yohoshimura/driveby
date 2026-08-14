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
    const handler = (e) => {
      if (e.key === 'Escape') onResolve(false);
      if (e.key === 'Enter') onResolve(true);
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
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
