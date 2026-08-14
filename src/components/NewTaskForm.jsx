import React, { useState } from 'react';
import Button from './common/Button';
import FormField from './common/FormField';
import { bridge } from '../lib/tauri';
import { useT } from '../hooks/useT';

const INITIAL = { name: '', source: '', destination: '', schedule: 'manual' };

export default function NewTaskForm({ onAdd, onSave, onCancel, defaultDestination, showToast, initialTask, dataState }) {
  const t = useT();
  const isEdit = !!initialTask;
  const [task, setTask] = useState(() =>
    initialTask
      ? {
          name: initialTask.name || '',
          source: initialTask.source || '',
          destination: initialTask.destination || '',
          schedule: initialTask.schedule || 'manual',
        }
      : INITIAL
  );

  const pick = async (type) => {
    const title = type === 'source' ? t('form.dialog.select_source') : t('form.dialog.select_destination');
    const p = await bridge.selectDirectory(title);
    if (p) setTask((prev) => ({ ...prev, [type]: p }));
  };

  const submit = () => {
    if (!task.name.trim()) return showToast?.(t('form.error.name'), 'error');
    if (!task.source) return showToast?.(t('form.error.source'), 'error');
    if (!task.destination && !defaultDestination) {
      return showToast?.(t('form.error.dest'), 'error');
    }
    if (isEdit) {
      onSave(task);
      return;
    }
    const ok = onAdd(task);
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
          <Button size="small" onClick={() => pick('source')}>{t('common.choose')}</Button>
        </div>
      </FormField>

      <FormField label={destinationLabel}>
        <div className="field-row">
          <input
            className="field field--readonly"
            readOnly
            value={task.destination || defaultDestination}
            placeholder={defaultDestination || t('form.placeholder.choose')}
            autoComplete="off"
            name="driveby-task-destination"
          />
          <Button size="small" onClick={() => pick('destination')}>{t('common.choose')}</Button>
        </div>
      </FormField>

      <FormField label={t('form.label.schedule')} hint={t('form.hint.schedule')}>
        <select
          className="field"
          value={task.schedule}
          onChange={(e) => setTask({ ...task, schedule: e.target.value })}
        >
          <option value="manual">{t('task.schedule.manual')}</option>
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
