import { useCallback } from 'react';
import { useT } from './useT';
import { useFormat } from './useFormat';

const SCHEDULE_KEY = {
  manual: 'task.schedule.manual',
  hourly: 'task.schedule.hourly',
  daily: 'task.schedule.daily',
  weekly: 'task.schedule.weekly',
  monthly: 'task.schedule.monthly',
  custom: 'task.schedule.custom',
};

/// How a task's schedule reads on a card: one word for the fixed ones,
/// "Mon, Thu · 22:00" for a custom one. Shared by the task list and the
/// statistics list, which showed the same line and drifted apart the moment
/// a sixth kind of schedule existed.
///
/// A custom schedule missing its days or its time falls back to the bare
/// word: the scheduler treats that task as inert, and a label pretending to
/// name a time would be describing a run that never happens.
export function useScheduleLabel() {
  const t = useT();
  const { formatWeekdays, formatClock } = useFormat();
  return useCallback((task) => {
    if (task?.schedule !== 'custom') {
      return t(SCHEDULE_KEY[task?.schedule] || SCHEDULE_KEY.manual);
    }
    const days = formatWeekdays(task.scheduleDays);
    const time = formatClock(task.scheduleTime);
    return days && time ? `${days} · ${time}` : t(SCHEDULE_KEY.custom);
  }, [t, formatWeekdays, formatClock]);
}
