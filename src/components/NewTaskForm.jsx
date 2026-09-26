import React, { useRef, useState } from 'react';
import Button from './common/Button';
import FormField from './common/FormField';
import { bridge } from '../lib/tauri';
import { useT } from '../hooks/useT';
import { useFormat } from '../hooks/useFormat';
import {
  findDuplicateFolder,
  findForeignOverlap,
  findOverlap,
  folderNameError,
  keepVersionsDays,
  pathContains,
  sourceFolderName,
  taskDestinations,
  taskSources,
  usesSubfolders,
  versionChoices,
  versionsAtRisk,
  versionsNeedChecking,
} from '../lib/task';
import {
  DEFAULT_SCHEDULE_TIME,
  WEEKDAY_INDEXES,
  nextOccurrence,
  normalizeDays,
} from '../lib/schedule';

const INITIAL = {
  name: '',
  sources: [],
  destinations: [],
  schedule: 'manual',
  scheduleDays: [],
  scheduleTime: DEFAULT_SCHEDULE_TIME,
  keepVersionsDays: 0,
};

export default function NewTaskForm({ onAdd, onSave, onCancel, defaultDestination, showToast, confirm, initialTask, dataState, otherTasks = [] }) {
  const t = useT();
  const { formatTime, formatWeekdays } = useFormat();
  const isEdit = !!initialTask;
  const [task, setTask] = useState(() =>
    initialTask
      ? {
          name: initialTask.name || '',
          sources: taskSources(initialTask),
          destinations: taskDestinations(initialTask),
          schedule: initialTask.schedule || 'manual',
          scheduleDays: normalizeDays(initialTask.scheduleDays),
          scheduleTime: initialTask.scheduleTime || DEFAULT_SCHEDULE_TIME,
          keepVersionsDays: keepVersionsDays(initialTask),
        }
      : INITIAL
  );
  // A save in flight (see `submit`), and a Cancel clicked while it was.
  const [saving, setSaving] = useState(false);
  const busy = useRef(false);
  const cancelled = useRef(false);

  /// Pick a folder into source slot `index`, or append it when `index` is
  /// null, named after itself.
  ///
  /// Refused at the moment of picking for the same reason destinations are:
  /// the message can then name what is wrong while it is still a choice. The
  /// backend refuses the same pairs on its own — a source nested in another
  /// would be backed up twice, into two folders.
  const pickSource = async (index) => {
    const picked = await bridge.selectDirectory(t('form.dialog.select_source'));
    if (!picked) return;
    const others = task.sources.filter((_, i) => i !== index);
    if (others.some((s) => pathContains(s.path, picked) || pathContains(picked, s.path))) {
      showToast?.(t('form.error.source_overlap'), 'error');
      return;
    }
    if (task.destinations.some((d) => pathContains(d, picked) || pathContains(picked, d))) {
      showToast?.(t('form.error.dest_in_source'), 'error');
      return;
    }
    setTask((prev) => {
      const sources = [...prev.sources];
      const next = { path: picked, folder: sourceFolderName(picked) };
      if (index === null || index >= sources.length) sources.push(next);
      else sources[index] = next;
      return { ...prev, sources };
    });
  };

  const setSourceFolder = (index, folder) =>
    setTask((prev) => ({
      ...prev,
      sources: prev.sources.map((s, i) => (i === index ? { ...s, folder } : s)),
    }));

  const removeSource = (index) =>
    setTask((prev) => ({
      ...prev,
      sources: prev.sources.filter((_, i) => i !== index),
    }));

  // Whether the sources get a folder each at the destination, which is what
  // decides if their folder names are shown and checked at all.
  const nested = usesSubfolders(task.sources);

  // Marked on the row as well as refused at submit: the folder name is the
  // one thing here typed by hand, and a drive root arrives with it empty.
  const folderInvalid = (index) => {
    const source = task.sources[index];
    return !!folderNameError(source.folder)
      || task.sources.some((other, i) => i !== index && findDuplicateFolder([source, other]));
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
    if (task.sources.some((s) => pathContains(s.path, picked) || pathContains(picked, s.path))) {
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

  const toggleDay = (day) =>
    setTask((prev) => ({
      ...prev,
      scheduleDays: prev.scheduleDays.includes(day)
        ? prev.scheduleDays.filter((d) => d !== day)
        : normalizeDays([...prev.scheduleDays, day]),
    }));

  // Null when the schedule cannot fire — no day picked, or a time the
  // backend will not parse. The form refuses to save in that state, and the
  // absent line is the first hint of why.
  const nextRun = task.schedule === 'custom'
    ? nextOccurrence(task.scheduleDays, task.scheduleTime)
    : null;

  const submit = async () => {
    if (busy.current) return;
    if (!task.name.trim()) return showToast?.(t('form.error.name'), 'error');
    if (task.sources.length === 0) return showToast?.(t('form.error.source'), 'error');
    // A custom schedule that cannot fire would leave a task looking
    // scheduled and never running. Refuse it here rather than let the
    // scheduler quietly treat it as manual.
    if (task.schedule === 'custom' && !nextRun) {
      return showToast?.(t('form.error.schedule'), 'error');
    }
    if (task.destinations.length === 0 && !defaultDestination) {
      return showToast?.(t('form.error.dest'), 'error');
    }
    // Re-checked at submit as well as at picking: the sources can be chosen
    // after the destinations, and an edited task can arrive here carrying a
    // pair an older version accepted.
    if (findOverlap(task.destinations)) return showToast?.(t('form.error.dest_overlap'), 'error');
    if (findOverlap(task.sources.map((s) => s.path))) {
      return showToast?.(t('form.error.source_overlap'), 'error');
    }
    if (task.destinations.some((d) => task.sources.some((s) => pathContains(s.path, d) || pathContains(d, s.path)))) {
      return showToast?.(t('form.error.dest_in_source'), 'error');
    }
    // The folder names the backend would refuse, with the message naming the
    // row: an empty name has nothing to quote but its path. Only asked of
    // several sources — a single one is mirrored straight in and its name is
    // never used.
    if (nested) {
      const misnamed = task.sources.find((s) => folderNameError(s.folder));
      if (misnamed) {
        const error = folderNameError(misnamed.folder);
        const key = error === 'empty'
          ? 'form.error.source_folder_empty'
          : error === 'reserved'
            ? 'form.error.source_folder_reserved'
            : 'form.error.source_folder_invalid';
        return showToast?.(t(key, { path: misnamed.path, folder: misnamed.folder.trim() }), 'error');
      }
      const duplicate = findDuplicateFolder(task.sources);
      if (duplicate) {
        return showToast?.(t('form.error.source_folder_duplicate', { folder: duplicate }), 'error');
      }
    }
    // Sharing a folder with another task is not sharing: each run mirror-prunes
    // the folder against its own source and deletes what the other just wrote,
    // reporting it as a clean-up. The backend refuses such a run outright; this
    // says so at the point the folder is picked, while it is still a choice.
    const foreign = findForeignOverlap(task.destinations, otherTasks);
    if (foreign) {
      const key = foreign.kind === 'source' ? 'form.error.dest_holds_source' : 'form.error.dest_foreign';
      return showToast?.(t(key, { name: foreign.name, path: foreign.path }), 'error');
    }
    // Folder names are stored trimmed, the way both sides read them, so
    // tasks.json says what the run will write.
    const named = { ...task, sources: task.sources.map((s) => ({ ...s, folder: s.folder.trim() })) };
    // Resolve the default here rather than leaving the list empty: an edit
    // saved with nothing picked used to store a blank destination, and the
    // task then failed at the next run instead of quietly using the default
    // the field was showing all along.
    const resolved = named.destinations.length > 0
      ? named
      : { ...named, destinations: defaultDestination ? [defaultDestination] : [] };
    // Listing the destinations waits on each drive — a disk spinning up
    // takes seconds. Meanwhile a second click must not add the task twice,
    // and a Cancel must not let the save go ahead once the form is gone.
    busy.current = true;
    cancelled.current = false;
    setSaving(true);
    try {
      if (confirm && !(await versionsConfirmed(resolved))) return;
      if (cancelled.current) return;
      if (isEdit) {
        onSave(resolved);
        return;
      }
      const ok = onAdd(resolved);
      if (ok) setTask(INITIAL);
    } finally {
      busy.current = false;
      setSaving(false);
    }
  };

  /// Fewer days, or none, deletes versions at the next run. Asked here,
  /// while it is still a choice: the run itself asks nobody. Asked of the
  /// days the destinations hold as well as of this task's own setting: a new
  /// or re-created task pointed at a destination that already keeps days
  /// deletes them just the same. A destination that is not plugged in lists
  /// nothing, so a setting that went down still asks on its own. An edit that
  /// changes neither the days nor the destinations keeps the retention the
  /// task already had, and is not asked about it again.
  const versionsConfirmed = async (resolved) => {
    const before = keepVersionsDays(initialTask);
    const after = task.keepVersionsDays;
    let risk = isEdit && before > 0 && after < before ? (after === 0 ? 'off' : 'fewer') : null;
    if (!risk && versionsNeedChecking(initialTask, resolved)) {
      const listed = await Promise.all(
        resolved.destinations.map((d) => bridge.listSnapshots(d).catch(() => [])),
      );
      if (cancelled.current) return false;
      risk = listed
        .map((days) => versionsAtRisk((days ?? []).map((day) => day?.name), after))
        .find(Boolean) ?? null;
    }
    if (!risk) return true;
    return confirm(risk === 'off'
      ? {
          title: t('form.versions.off_confirm.title'),
          body: t('form.versions.off_confirm.body'),
          confirmLabel: t('form.versions.off_confirm.action'),
          danger: true,
        }
      : {
          title: t('form.versions.fewer_confirm.title'),
          body: t('form.versions.fewer_confirm.body', { n: after, count: after }),
          confirmLabel: t('form.versions.fewer_confirm.action'),
          danger: true,
        });
  };

  const cancel = () => {
    cancelled.current = true;
    onCancel();
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

      <FormField
        label={t('form.label.sources')}
        hint={t(nested ? 'form.hint.sources' : 'form.hint.source_single')}
      >
        <div className="dest-list">
          {task.sources.length === 0 ? (
            <div className="field-row">
              <input
                className="field field--readonly"
                readOnly
                value=""
                placeholder={t('form.placeholder.choose')}
                aria-label={t('form.label.sources')}
                autoComplete="off"
                name="driveby-task-source"
              />
              <Button size="small" onClick={() => pickSource(null)}>{t('common.choose')}</Button>
            </div>
          ) : (
            task.sources.map((source, i) => (
              <div className="field-row" key={`${i}-${source.path}`}>
                <input
                  className="field field--readonly"
                  readOnly
                  value={source.path}
                  title={source.path}
                  aria-label={t('form.aria.source', { n: i + 1 })}
                  autoComplete="off"
                  name={`driveby-task-source-${i}`}
                />
                <Button size="small" onClick={() => pickSource(i)}>{t('common.choose')}</Button>
                {nested && (
                  <input
                    type="text"
                    className="field field--folder"
                    value={source.folder}
                    onChange={(e) => setSourceFolder(i, e.target.value)}
                    placeholder={t('form.placeholder.source_folder')}
                    aria-label={t('form.aria.source_folder', { n: i + 1 })}
                    aria-invalid={folderInvalid(i)}
                    autoComplete="off"
                    autoCorrect="off"
                    autoCapitalize="off"
                    spellCheck={false}
                    name={`driveby-task-source-folder-${i}`}
                  />
                )}
                <Button
                  size="small"
                  variant="borderless"
                  destructive
                  onClick={() => removeSource(i)}
                  ariaLabel={t('form.aria.remove_source', { path: source.path })}
                >
                  {t('form.action.remove_source')}
                </Button>
              </div>
            ))
          )}
          {task.sources.length > 0 && (
            <Button size="small" variant="borderless" onClick={() => pickSource(null)}>
              {t('form.action.add_source')}
            </Button>
          )}
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
          <option value="custom">{t('task.schedule.custom')}</option>
        </select>
      </FormField>

      {task.schedule === 'custom' && (
        <FormField
          label={t('form.label.schedule_days')}
          hint={nextRun
            ? t('form.hint.next_run', { when: formatTime(nextRun.toISOString()) })
            : t('form.hint.no_next_run')}
        >
          <div className="day-picker">
            <div role="group" aria-label={t('form.label.schedule_days')} className="day-picker__days">
              {WEEKDAY_INDEXES.map((day) => {
                const on = task.scheduleDays.includes(day);
                const name = formatWeekdays([day]);
                return (
                  <button
                    key={day}
                    type="button"
                    role="checkbox"
                    aria-checked={on}
                    aria-label={name}
                    className={`day-picker__day ${on ? 'day-picker__day--on' : ''}`}
                    onClick={() => toggleDay(day)}
                  >
                    {name}
                  </button>
                );
              })}
            </div>
            <input
              type="time"
              className="field field--compact day-picker__time"
              value={task.scheduleTime}
              onChange={(e) => setTask({ ...task, scheduleTime: e.target.value })}
              aria-label={t('form.label.schedule_time')}
              name="driveby-schedule-time"
            />
          </div>
        </FormField>
      )}

      <FormField label={t('form.label.versions')} hint={t('form.hint.versions')}>
        <select
          className="field"
          value={task.keepVersionsDays}
          onChange={(e) => setTask({ ...task, keepVersionsDays: Number(e.target.value) })}
        >
          {versionChoices(keepVersionsDays(initialTask)).map((days) => (
            <option key={days} value={days}>
              {days === 0
                ? t('form.versions.off')
                : days === 365
                  ? t('form.versions.year')
                  : t('form.versions.days', { n: days, count: days })}
            </option>
          ))}
        </select>
      </FormField>

      <div className="card__actions">
        <Button onClick={cancel}>{t('common.cancel')}</Button>
        <Button variant="primary" onClick={submit} disabled={saving}>
          {isEdit ? t('form.action.save') : t('form.action.add')}
        </Button>
      </div>
    </div>
  );
}
