import React, { useState, useMemo, useRef } from 'react';
import { useApp } from '../context/AppContext';
import Button from './common/Button';
import EmptyState from './common/EmptyState';
import TaskCard from './TaskCard';
import NewTaskForm from './NewTaskForm';
import { useKeyboard } from '../hooks/useKeyboard';
import { useExitTransition } from '../hooks/useExitTransition';
import { useT } from '../hooks/useT';

export default function Home() {
  const {
    tasks, activeBackups, settings,
    startBackup, cancelBackup, addTask, editTask, deleteTask, showToast,
  } = useApp();
  const t = useT();
  const [showForm, setShowForm] = useState(false);
  const [editingId, setEditingId] = useState(null);

  const closeForm = () => { setShowForm(false); setEditingId(null); };

  useKeyboard(useMemo(() => [
    { key: 'n', ctrl: true, handler: () => { setEditingId(null); setShowForm((s) => !s); } },
    { key: 'Escape', handler: () => closeForm() },
  ], []));

  const editingTask = editingId ? tasks.find((t) => t.id === editingId) : null;
  const formOpen = showForm || !!editingTask;
  const { mounted: formMounted, state: formAnim } = useExitTransition(formOpen, 220);
  const lastEditingRef = useRef(editingTask);
  if (editingTask) lastEditingRef.current = editingTask;
  const renderEditing = editingTask || (formAnim === 'exit' ? lastEditingRef.current : null);

  return (
    <>
      <div className="section-head">
        <h2 className="title-2">{t('view.tasks')}</h2>
        <Button
          variant="primary"
          size="small"
          onClick={() => {
            if (showForm) { closeForm(); } else { setEditingId(null); setShowForm(true); }
          }}
        >
          {showForm ? t('common.cancel') : t('home.new_task')}
        </Button>
      </div>

      {formMounted && (
        <NewTaskForm
          key={renderEditing ? renderEditing.id : 'new'}
          initialTask={renderEditing}
          dataState={formAnim}
          onAdd={(draft) => {
            const ok = addTask(draft);
            if (ok) {
              closeForm();
              showToast(t('task.toast.added'));
            }
            return ok;
          }}
          onSave={(draft) => {
            editTask(renderEditing.id, draft);
            closeForm();
            showToast(t('task.toast.updated'));
            return true;
          }}
          onCancel={closeForm}
          defaultDestination={settings.defaultDestination}
          showToast={showToast}
        />
      )}

      {tasks.length === 0 ? (
        <EmptyState title={t('common.empty')} />
      ) : (
        <div className="group" role="list">
          {tasks.map((task, i) => (
            <TaskCard
              key={task.id}
              task={task}
              index={i}
              backup={activeBackups[task.id]}
              onRun={startBackup}
              onCancel={cancelBackup}
              onModify={(t) => { setShowForm(false); setEditingId(t.id); }}
              onDelete={deleteTask}
            />
          ))}
        </div>
      )}
    </>
  );
}
