import React from 'react';
import Button from './common/Button';
import { useFormat } from '../hooks/useFormat';
import { useT } from '../hooks/useT';
import { taskDestinations } from '../lib/task';
import { useScheduleLabel } from '../hooks/useScheduleLabel';

export default function TaskCard({ task, backup, missing, onRun, onCancel, onModify, onDelete, index = 0 }) {
  const t = useT();
  const { formatTime } = useFormat();
  const isRunning = !!backup;
  const scheduleLabel = useScheduleLabel()(task);
  const lastRun = task.lastBackup ? formatTime(task.lastBackup) : t('common.never');
  const paths = `${task.source} → ${taskDestinations(task).join(', ')}`;
  // Destinations are written one after another, so the bar restarts at zero
  // for each of them. The counter is what explains that; it goes on the meta
  // line because the running card deliberately has nothing under the bar.
  const stepping = isRunning && backup.destCount > 1;
  return (
    <article className="task" style={{ '--stagger': `${Math.min(index, 8) * 40}ms` }}>
      <div className="task__body">
        <div className="task__name">{task.name}</div>
        <div className="task__paths" title={paths}>{paths}</div>
        <div className="task__meta">
          {t('task.last_run', { time: lastRun, schedule: scheduleLabel })}
          {stepping
            && ` · ${t('task.running.destination', {
              index: backup.destIndex + 1,
              total: backup.destCount,
            })}`}
        </div>

        {/* Whether a drive is there is checked once a minute by the
            scheduler, so this stays honest without the card polling for
            it. The paths themselves go in the tooltip: on a task with
            three destinations, which one is missing is the question. */}
        {missing?.length > 0 && (
          <div className="task__missing" title={missing.join('\n')}>
            {t('task.destination_missing', { n: missing.length, count: missing.length })}
          </div>
        )}

        {isRunning && (
          <div className="task__progress">
            <div
              className="progressbar"
              role="progressbar"
              aria-label={t('task.aria.progress', { name: task.name })}
              aria-valuenow={backup.progress || 0}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <div className="progressbar__fill" style={{ width: `${backup.progress || 0}%` }} />
            </div>
          </div>
        )}
      </div>

      <div className="task__actions">
        {!isRunning ? (
          <Button
            variant="primary"
            size="small"
            onClick={() => onRun(task)}
            ariaLabel={t('task.aria.run', { name: task.name })}
          >
            {t('task.action.backup')}
          </Button>
        ) : (
          <Button
            size="small"
            destructive
            onClick={() => onCancel(task.id)}
            ariaLabel={t('task.aria.cancel', { name: task.name })}
          >
            {t('task.action.stop')}
          </Button>
        )}
        <Button
          size="small"
          variant="borderless"
          onClick={() => onModify(task)}
          ariaLabel={t('task.aria.modify', { name: task.name })}
          disabled={isRunning}
        >
          {t('task.action.modify')}
        </Button>
        <Button
          size="small"
          variant="borderless"
          onClick={() => onDelete(task.id)}
          ariaLabel={t('task.aria.delete', { name: task.name })}
        >
          {t('task.action.delete')}
        </Button>
      </div>
    </article>
  );
}
