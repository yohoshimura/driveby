import React, { useRef } from 'react';
import { useExitTransition } from '../hooks/useExitTransition';

export default function Toast({ toast }) {
  const lastRef = useRef(toast);
  if (toast) lastRef.current = toast;

  const { mounted, state } = useExitTransition(!!toast, 220);
  if (!mounted) return null;
  const t = toast || lastRef.current;
  if (!t) return null;

  return (
    <div
      key={t.id}
      className={`toast ${t.kind === 'error' ? 'toast--error' : ''}`}
      role="status"
      aria-live="polite"
      data-state={state}
    >
      {t.message}
    </div>
  );
}
