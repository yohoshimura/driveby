import React from 'react';
import { useApp } from '../context/AppContext';
import { useT } from '../hooks/useT';
import CandleChart from './charts/CandleChart';
import TaskList from './charts/TaskList';
import GroupedBarChart from './charts/GroupedBarChart';

export default function Statistics() {
  const { tasks, history } = useApp();
  const t = useT();

  return (
    <>
      <div className="stat-block" style={{ '--stat-delay': '0ms' }}>
        <div className="stat-block__head">{t('statistics.backed_up')}</div>
        <CandleChart entries={history} />
      </div>

      <div className="stat-block" style={{ '--stat-delay': '90ms' }}>
        <div className="stat-block__head">{t('statistics.tasks')}</div>
        <TaskList tasks={tasks} />
      </div>

      <div className="stat-block" style={{ '--stat-delay': '180ms' }}>
        <div className="stat-block__head">{t('statistics.successful_runs')}</div>
        <GroupedBarChart tasks={tasks} history={history} />
      </div>
    </>
  );
}
