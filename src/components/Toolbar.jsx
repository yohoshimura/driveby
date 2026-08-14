import React from 'react';
import { PanelLeft } from 'lucide-react';
import { useT } from '../hooks/useT';

export default function Toolbar({ title, sidebarOpen, onToggleSidebar, rightSlot }) {
  const t = useT();
  return (
    <header className="toolbar">
      <button
        type="button"
        className="toolbar__toggle"
        onClick={onToggleSidebar}
        aria-label={sidebarOpen ? t('toolbar.toggle.hide') : t('toolbar.toggle.show')}
        aria-pressed={sidebarOpen}
      >
        <PanelLeft size={15} />
      </button>
      <div className="toolbar__title">{title}</div>
      <div className="toolbar__right">{rightSlot}</div>
    </header>
  );
}
