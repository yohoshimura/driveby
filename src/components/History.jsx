import React, { useState, useMemo } from 'react';
import { useApp } from '../context/AppContext';
import { formatTime, formatBytes, formatDuration } from '../lib/format';
import Button from './common/Button';
import EmptyState from './common/EmptyState';
import { useT } from '../hooks/useT';

const STATUS_KEY = {
  success: 'history.status.success',
  cancelled: 'history.status.cancelled',
  error: 'history.status.error',
};

export default function History() {
  const { history, deleteHistory, clearHistory, revealFolder, restoreBackup } = useApp();
  const t = useT();
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');

  const filtered = useMemo(() => {
    return history.filter((h) => {
      if (filter !== 'all' && h.status !== filter) return false;
      if (!query) return true;
      const q = query.toLowerCase();
      return (
        h.taskName?.toLowerCase().includes(q) ||
        h.path?.toLowerCase().includes(q) ||
        h.error?.toLowerCase().includes(q)
      );
    });
  }, [history, query, filter]);

  if (history.length === 0) return <EmptyState title={t('common.empty')} />;

  return (
    <section>
      <div className="section-head">
        <h2 className="title-2">{t('history.title')}</h2>
        <Button size="small" variant="borderless" onClick={clearHistory}>{t('history.clear_all')}</Button>
      </div>

      <div className="history-toolbar">
        <input
          className="field field--compact"
          type="search"
          placeholder={t('history.search')}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label={t('history.search')}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          name="driveby-history-search"
        />
        <select
          className="field field--compact"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          aria-label={t('history.filter.aria')}
          style={{ width: 120 }}
        >
          <option value="all">{t('history.filter.all')}</option>
          <option value="success">{t('history.filter.success')}</option>
          <option value="error">{t('history.filter.errors')}</option>
          <option value="cancelled">{t('history.filter.cancelled')}</option>
        </select>
      </div>

      <table className="history-table">
        <thead>
          <tr>
            <th>{t('history.col.date')}</th>
            <th>{t('history.col.task')}</th>
            <th>{t('history.col.status')}</th>
            <th className="th--right">{t('history.col.size')}</th>
            <th className="th--right">{t('history.col.files')}</th>
            <th className="th--right">{t('history.col.duration')}</th>
            <th className="th--right">{t('history.col.actions')}</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((entry) => (
            <tr key={entry.id}>
              <td><span className="mono">{formatTime(entry.timestamp)}</span></td>
              <td>
                <div className="history-task-name">{entry.taskName}</div>
                {entry.path && <div className="history-path" title={entry.path}>{entry.path}</div>}
                {entry.error && <div className="history-path" style={{ color: 'var(--system-red)' }}>{entry.error}</div>}
                {entry.unreadable > 0 && (
                  <div className="history-path" style={{ color: 'var(--system-orange)' }}>
                    {t('history.unreadable', { n: entry.unreadable })}
                  </div>
                )}
              </td>
              <td>
                <span className={`badge badge--${entry.status}`}>
                  {t(STATUS_KEY[entry.status] || STATUS_KEY.error)}
                </span>
              </td>
              <td className="td--right"><span className="mono">{entry.totalBytes ? formatBytes(entry.totalBytes) : t('common.dash')}</span></td>
              <td className="td--right"><span className="mono">{entry.totalFiles ?? t('common.dash')}</span></td>
              <td className="td--right"><span className="mono">{entry.durationMs ? formatDuration(Math.round(entry.durationMs / 1000)) : t('common.dash')}</span></td>
              <td className="td--right">
                {entry.path && entry.status === 'success' && (
                  <Button size="small" variant="borderless" onClick={() => restoreBackup(entry.path)}>{t('common.restore')}</Button>
                )}
                {entry.path && (
                  <Button size="small" variant="borderless" onClick={() => revealFolder(entry.path)}>{t('common.reveal')}</Button>
                )}
                <Button size="small" variant="borderless" onClick={() => deleteHistory(entry.id)}>{t('common.delete')}</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
