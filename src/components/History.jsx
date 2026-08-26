import React, { useState, useMemo } from 'react';
import { useApp } from '../context/AppContext';
import { useProgress } from '../context/ProgressContext';
import { useFormat } from '../hooks/useFormat';
import Button from './common/Button';
import EmptyState from './common/EmptyState';
import { useT } from '../hooks/useT';
import { destinationsOf } from '../lib/history';

const STATUS_KEY = {
  success: 'history.status.success',
  partial: 'history.status.partial',
  cancelled: 'history.status.cancelled',
  error: 'history.status.error',
};

// One glyph per destination state. The row badge already carries the run's
// overall word; repeating it on every line underneath would drown the paths,
// which are what the reader is actually scanning for.
const DEST_MARK = {
  success: '✓',
  error: '✗',
  unreachable: '⚠',
  cancelled: '–',
};

const DEST_STATUS_KEY = {
  success: 'history.dest.success',
  error: 'history.dest.error',
  unreachable: 'history.dest.unreachable',
  cancelled: 'history.dest.cancelled',
};

export default function History() {
  const { history, deleteHistory, clearHistory, revealFolder, restoreBackup } = useApp();
  const { activeRestore } = useProgress();
  const t = useT();
  const { formatTime, formatBytes, formatDuration, formatNumber } = useFormat();
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');

  const filtered = useMemo(() => {
    return history.filter((h) => {
      if (filter !== 'all' && h.status !== filter) return false;
      if (!query) return true;
      const q = query.toLowerCase();
      return (
        h.taskName?.toLowerCase().includes(q) ||
        // Every destination is searchable, not just the first one that
        // succeeded — otherwise the drive you are looking for is the one
        // you cannot find.
        destinationsOf(h).some((d) => d.path?.toLowerCase().includes(q)) ||
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
          <option value="partial">{t('history.filter.partial')}</option>
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
                {destinationsOf(entry).map((dest, i) => {
                  const status = DEST_STATUS_KEY[dest.status] ? dest.status : 'error';
                  const label = t(DEST_STATUS_KEY[status]);
                  return (
                    <div className="history-dest" key={`${i}-${dest.path}`}>
                      <span
                        className={`history-dest__mark history-dest__mark--${status}`}
                        role="img"
                        aria-label={label}
                        title={label}
                      >
                        {DEST_MARK[status]}
                      </span>
                      <span className="history-path" title={dest.path}>{dest.path}</span>
                      {status === 'success' && (
                        <Button
                          size="small"
                          variant="borderless"
                          onClick={() => restoreBackup(dest.path)}
                          disabled={!!activeRestore}
                        >
                          {t('common.restore')}
                        </Button>
                      )}
                      {/* No Reveal for a destination that was never written:
                          the folder it names is not there to open. */}
                      {(status === 'success' || status === 'error') && (
                        <Button size="small" variant="borderless" onClick={() => revealFolder(dest.path)}>
                          {t('common.reveal')}
                        </Button>
                      )}
                      {dest.error && <span className="history-dest__error">{dest.error}</span>}
                    </div>
                  );
                })}
                {/* The run-level error only when no destination carried one
                    of its own — a missing source, an unsafe configuration. */}
                {entry.error && destinationsOf(entry).length === 0 && (
                  <div className="history-path" style={{ color: 'var(--system-red)' }}>{entry.error}</div>
                )}
                {entry.unreadable > 0 && (
                  <div className="history-path" style={{ color: 'var(--system-orange)' }}>
                    {t('history.unreadable', { n: entry.unreadable, count: entry.unreadable })}
                  </div>
                )}
              </td>
              <td>
                <span className={`badge badge--${entry.status}`}>
                  {t(STATUS_KEY[entry.status] || STATUS_KEY.error)}
                </span>
              </td>
              <td className="td--right"><span className="mono">{entry.totalBytes ? formatBytes(entry.totalBytes) : t('common.dash')}</span></td>
              <td className="td--right"><span className="mono">{entry.totalFiles != null ? formatNumber(entry.totalFiles) : t('common.dash')}</span></td>
              <td className="td--right"><span className="mono">{entry.durationMs ? formatDuration(Math.round(entry.durationMs / 1000)) : t('common.dash')}</span></td>
              <td className="td--right">
                <Button size="small" variant="borderless" onClick={() => deleteHistory(entry.id)}>{t('common.delete')}</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
