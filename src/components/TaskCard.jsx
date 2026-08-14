import React from 'react';
import Button from './common/Button';
import { formatTime } from '../lib/format';
import { useT } from '../hooks/useT';

const SCHEDULE_KEY = {
  manual: 'task.schedule.manual',
  daily: 'task.schedule.daily',
  weekly: 'task.schedule.weekly',
  monthly: 'task.schedule.monthly',
};

export default function TaskCard({ task, backup, onRun, onCancel, onModify, onDelete, index = 0 }) {
  const t = useT();
  const isRunning = !!backup;
  const scheduleLabel = t(SCHEDULE_KEY[task.schedule] || 'task.schedule.manual');
  const lastRun = task.lastBackup ? formatTime(task.lastBackup) : t('common.never');
  return (
    <article className="task" style={{ '--stagger': `${Math.min(index, 8) * 40}ms` }}>
      <div className="task__body">
        <div className="task__name">{task.name}</div>
        <div className="task__paths" title={`${task.source} → ${task.destination}`}>
          {task.source} → {task.destination}
        </div>
        <div className="task__meta">
          {t('task.last_run', { time: lastRun, schedule: scheduleLabel })}
        </div>

        {isRunning && (
          <div className="task__progress" aria-live="polite">
            <div
              className="progressbar"
              role="progressbar"
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
