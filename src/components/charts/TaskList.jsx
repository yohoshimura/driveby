import React from 'react';
import { useFormat } from '../../hooks/useFormat';
import { useT } from '../../hooks/useT';
import { taskDestinations } from '../../lib/task';
import { useScheduleLabel } from '../../hooks/useScheduleLabel';

export default function TaskList({ tasks }) {
  const t = useT();
  const { formatTime } = useFormat();
  const scheduleLabelFor = useScheduleLabel();
  if (!tasks || tasks.length === 0) {
    return <div className="chart-empty">{t('chart.empty.tasks')}</div>;
  }
  return (
    <div className="task-list-stat">
      <ul className="task-list-stat__items" role="list">
        {tasks.map((task, i) => {
          const lastRun = task.lastBackup ? formatTime(task.lastBackup) : t('common.never');
          const scheduleLabel = scheduleLabelFor(task);
          const paths = `${task.source} → ${taskDestinations(task).join(', ')}`;
          return (
            <li
              key={task.id}
              className="task-list-stat__item"
              style={{ '--stagger': `${Math.min(i, 8) * 35}ms` }}
            >
              <div className="task-list-stat__name">{task.name}</div>
              <div className="task-list-stat__meta" title={paths}>{paths}</div>
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
