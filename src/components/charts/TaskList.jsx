import React from 'react';
import { formatTime } from '../../lib/format';
import { useT } from '../../hooks/useT';

const SCHEDULE_KEY = {
  manual: 'task.schedule.manual',
  daily: 'task.schedule.daily',
  weekly: 'task.schedule.weekly',
  monthly: 'task.schedule.monthly',
};

export default function TaskList({ tasks }) {
  const t = useT();
  if (!tasks || tasks.length === 0) {
    return <div className="chart-empty">{t('chart.empty.tasks')}</div>;
  }
  return (
    <div className="task-list-stat">
      <ul className="task-list-stat__items" role="list">
        {tasks.map((task, i) => {
          const lastRun = task.lastBackup ? formatTime(task.lastBackup) : t('common.never');
          const scheduleLabel = t(SCHEDULE_KEY[task.schedule] || 'task.schedule.manual');
          return (
            <li
              key={task.id}
              className="task-list-stat__item"
              style={{ '--stagger': `${Math.min(i, 8) * 35}ms` }}
            >
              <div className="task-list-stat__name">{task.name}</div>
              <div className="task-list-stat__meta" title={`${task.source} → ${task.destination}`}>
                {task.source} → {task.destination}
              </div>
              <div className="task-list-stat__sub">
                {t('task.last_run', { time: lastRun, schedule: scheduleLabel })}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
