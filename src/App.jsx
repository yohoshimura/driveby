import React, { useState, useEffect, useMemo } from 'react';
import { AppProvider, useApp } from './context/AppContext';
import { ProgressProvider } from './context/ProgressContext';
import { useKeyboard } from './hooks/useKeyboard';
import Sidebar from './components/Sidebar';
import Toolbar from './components/Toolbar';
import Home from './components/Home';
import History from './components/History';
import Settings from './components/Settings';
import Statistics from './components/Statistics';
import ConfirmDialog from './components/ConfirmDialog';
import RestoreOverlay from './components/RestoreOverlay';
import Toast from './components/Toast';
import { checkForUpdate } from './lib/updater';
import { useT } from './hooks/useT';

const TITLE_KEYS = {
  home: 'view.tasks',
  history: 'view.history',
  statistics: 'view.statistics',
  settings: 'view.settings',
};

export default function App() {
  // ProgressProvider wraps AppProvider, not the other way round: it depends
  // on nothing, while AppContext needs it to flag a restore as in flight.
  return (
    <ProgressProvider>
      <AppProvider>
        <Shell />
      </AppProvider>
    </ProgressProvider>
  );
}

function Shell() {
  const { loaded, toast, confirmState, handleConfirm, settings, updateSetting, showToast } = useApp();
  const t = useT();
  const [view, setView] = useState(() => settings.lastView || 'home');
  const [sidebarOpen, setSidebarOpen] = useState(() => settings.sidebarOpen !== false);

  // Sync initial values once settings load
  useEffect(() => {
    if (settings.lastView && settings.lastView !== view) setView(settings.lastView);
    if (typeof settings.sidebarOpen === 'boolean') setSidebarOpen(settings.sidebarOpen);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  // Persist view + sidebar state on change
  useEffect(() => {
    if (!loaded) return;
    if (settings.lastView !== view) updateSetting('lastView', view);
  }, [view, loaded]);
  useEffect(() => {
    if (!loaded) return;
    if (settings.sidebarOpen !== sidebarOpen) updateSetting('sidebarOpen', sidebarOpen);
  }, [sidebarOpen, loaded]);

  // One silent check at launch — it only ever raises a toast pointing at
  // Settings; downloading and installing stays an explicit user action.
  useEffect(() => {
    if (!loaded || settings.checkUpdatesOnStart === false) return;
    let alive = true;
    checkForUpdate().then((res) => {
      if (alive && res.available) showToast(t('updates.toast.available'));
    });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  // Memoise so the keydown listener isn't re-attached on every render (#10).
  // Setters from useState are stable, so an empty dep list is fine.
  useKeyboard(useMemo(() => [
    { key: ',', ctrl: true, handler: () => setView('settings') },
    { key: '1', ctrl: true, handler: () => setView('home') },
    { key: '2', ctrl: true, handler: () => setView('history') },
    { key: '3', ctrl: true, handler: () => setView('statistics') },
    { key: '4', ctrl: true, handler: () => setView('settings') },
    { key: 's', ctrl: true, handler: () => setSidebarOpen((v) => !v) },
  ], []));

  if (!loaded) return <div className="app-loading" role="status">{t('app.loading')}</div>;

  return (
    <div className={`app ${sidebarOpen ? 'app--sidebar-open' : 'app--sidebar-closed'}`}>
      <Sidebar view={view} setView={setView} open={sidebarOpen} />
      <div className="main">
        <Toolbar
          title={t(TITLE_KEYS[view])}
          sidebarOpen={sidebarOpen}
          onToggleSidebar={() => setSidebarOpen((v) => !v)}
        />
        <main className="content">
          <div className="content__inner">
            <div className="view-route" key={view}>
              {view === 'home' && <Home />}
              {view === 'history' && <History />}
              {view === 'statistics' && <Statistics />}
              {view === 'settings' && <Settings />}
            </div>
          </div>
        </main>
      </div>
      <ConfirmDialog state={confirmState} onResolve={handleConfirm} />
      <RestoreOverlay />
      <Toast toast={toast} />
    </div>
  );
}
