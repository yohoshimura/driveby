import React from 'react';
import { useT } from '../../hooks/useT';

export default function EmptyState({ title, hint, action }) {
  const t = useT();
  const resolvedTitle = title ?? t('common.empty');
  return (
    <div className="empty">
      {resolvedTitle && <div className="empty__title">{resolvedTitle}</div>}
      {hint && <div className="empty__hint">{hint}</div>}
      {action && <div style={{ marginTop: 16 }}>{action}</div>}
    </div>
  );
}
