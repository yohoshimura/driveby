import React, { useState } from 'react';
import Button from './common/Button';
import FormField from './common/FormField';
import { bridge } from '../lib/tauri';
import { useT } from '../hooks/useT';
import { findOverlap, pathContains, taskDestinations } from '../lib/task';

const INITIAL = { name: '', source: '', destinations: [], schedule: 'manual' };

export default function NewTaskForm({ onAdd, onSave, onCancel, defaultDestination, showToast, initialTask, dataState }) {
  const t = useT();
  const isEdit = !!initialTask;
  const [task, setTask] = useState(() =>
    initialTask
      ? {
          name: initialTask.name || '',
          source: initialTask.source || '',
          destinations: taskDestinations(initialTask),
          schedule: initialTask.schedule || 'manual',
        }
      : INITIAL
  );

  const pickSource = async () => {
    const p = await bridge.selectDirectory(t('form.dialog.select_source'));
    if (p) setTask((prev) => ({ ...prev, source: p }));
  };

  /// Pick a folder into slot `index`, or append it when `index` is null.
  ///
  /// The overlap rules are enforced here, at the moment of picking, rather
  /// than only at submit: the message can then name the folder that is the
  /// problem. The backend refuses the same pairs on its own — it has to,
  /// since a destination nested inside another is pruned away by its host.
  const pickDestination = async (index) => {
    const picked = await bridge.selectDirectory(t('form.dialog.select_destination'));
    if (!picked) return;
    const others = task.destinations.filter((_, i) => i !== index);
    if (others.some((d) => pathContains(d, picked) || pathContains(picked, d))) {
      showToast?.(t('form.error.dest_overlap'), 'error');
      return;
    }
    if (task.source && (pathContains(task.source, picked) || pathContains(picked, task.source))) {
      showToast?.(t('form.error.dest_in_source'), 'error');
      return;
    }
    setTask((prev) => {
      const destinations = [...prev.destinations];
      if (index === null || index >= destinations.length) destinations.push(picked);
      else destinations[index] = picked;
      return { ...prev, destinations };
    });
  };

  const removeDestination = (index) =>
    setTask((prev) => ({
      ...prev,
      destinations: prev.destinations.filter((_, i) => i !== index),
    }));

  const submit = () => {
    if (!task.name.trim()) return showToast?.(t('form.error.name'), 'error');
    if (!task.source) return showToast?.(t('form.error.source'), 'error');
    if (task.destinations.length === 0 && !defaultDestination) {
      return showToast?.(t('form.error.dest'), 'error');
    }
    // Re-checked at submit as well as at picking: the source can be chosen
    // after the destinations, and an edited task can arrive here carrying a
    // pair an older version accepted.
    if (findOverlap(task.destinations)) return showToast?.(t('form.error.dest_overlap'), 'error');
    if (task.destinations.some((d) => pathContains(task.source, d) || pathContains(d, task.source))) {
      return showToast?.(t('form.error.dest_in_source'), 'error');
    }
    // Resolve the default here rather than leaving the list empty: an edit
    // saved with nothing picked used to store a blank destination, and the
    // task then failed at the next run instead of quietly using the default
    // the field was showing all along.
    const resolved = task.destinations.length > 0
      ? task
      : { ...task, destinations: defaultDestination ? [defaultDestination] : [] };
    if (isEdit) {
      onSave(resolved);
      return;
    }
    const ok = onAdd(resolved);
    if (ok) setTask(INITIAL);
  };

  const destinationLabel = defaultDestination
    ? t('form.label.destination_default')
    : t('form.label.destination');

  return (
    <div className="card" data-state={dataState}>
      <div className="card__head">{isEdit ? t('form.title.edit') : t('form.title.new')}</div>

      <FormField label={t('form.label.name')} htmlFor="task-name">
        <input
          id="task-name"
          type="text"
          className="field"
          value={task.name}
          onChange={(e) => setTask({ ...task, name: e.target.value })}
          placeholder={t('form.placeholder.name')}
          autoFocus
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          name="driveby-task-name"
        />
      </FormField>

      <FormField label={t('form.label.source')}>
        <div className="field-row">
          <input
            className="field field--readonly"
            readOnly
            value={task.source}
            placeholder={t('form.placeholder.choose')}
            autoComplete="off"
            name="driveby-task-source"
          />
          <Button size="small" onClick={pickSource}>{t('common.choose')}</Button>
        </div>
      </FormField>

      <FormField label={destinationLabel} hint={t('form.hint.destinations')}>
        <div className="dest-list">
          {task.destinations.length === 0 ? (
            <div className="field-row">
              <input
                className="field field--readonly"
                readOnly
                value=""
                placeholder={defaultDestination || t('form.placeholder.choose')}
                aria-label={destinationLabel}
                autoComplete="off"
                name="driveby-task-destination"
              />
              <Button size="small" onClick={() => pickDestination(null)}>{t('common.choose')}</Button>
            </div>
          ) : (
            task.destinations.map((dest, i) => (
              <div className="field-row" key={`${i}-${dest}`}>
                <input
                  className="field field--readonly"
                  readOnly
                  value={dest}
                  title={dest}
                  aria-label={t('form.aria.destination', { n: i + 1 })}
                  autoComplete="off"
                  name={`driveby-task-destination-${i}`}
                />
                <Button size="small" onClick={() => pickDestination(i)}>{t('common.choose')}</Button>
                <Button
                  size="small"
                  variant="borderless"
                  destructive
                  onClick={() => removeDestination(i)}
                  ariaLabel={t('form.aria.remove_destination', { path: dest })}
                >
                  {t('form.action.remove_destination')}
                </Button>
              </div>
            ))
          )}
          {task.destinations.length > 0 && (
            <Button size="small" variant="borderless" onClick={() => pickDestination(null)}>
              {t('form.action.add_destination')}
            </Button>
          )}
        </div>
      </FormField>

      <FormField label={t('form.label.schedule')} hint={t('form.hint.schedule')}>
        <select
          className="field"
          value={task.schedule}
          onChange={(e) => setTask({ ...task, schedule: e.target.value })}
        >
          <option value="manual">{t('task.schedule.manual')}</option>
          <option value="hourly">{t('task.schedule.hourly')}</option>
          <option value="daily">{t('task.schedule.daily')}</option>
          <option value="weekly">{t('task.schedule.weekly')}</option>
          <option value="monthly">{t('task.schedule.monthly')}</option>
        </select>
      </FormField>

      <div className="card__actions">
        <Button onClick={onCancel}>{t('common.cancel')}</Button>
        <Button variant="primary" onClick={submit}>
          {isEdit ? t('form.action.save') : t('form.action.add')}
        </Button>
      </div>
    </div>
  );
}
