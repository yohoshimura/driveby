import React, { useState, useMemo } from 'react';
import { Search, FolderSync, Clock, Settings as Cog, HardDrive, BarChart3 } from 'lucide-react';
import { useT } from '../hooks/useT';

// Static structure — labels are looked up via i18n at render time so the
// sidebar reflects the current language without a remount.
const NAV = [
  {
    sectionKey: 'sidebar.section.library',
    items: [
      { id: 'home', labelKey: 'view.tasks', icon: FolderSync },
      { id: 'history', labelKey: 'view.history', icon: Clock },
      { id: 'statistics', labelKey: 'view.statistics', icon: BarChart3 },
    ],
  },
  {
    sectionKey: 'sidebar.section.application',
    items: [
      { id: 'settings', labelKey: 'view.settings', icon: Cog },
    ],
  },
];

export default function Sidebar({ view, setView, open = true }) {
  const t = useT();
  const [query, setQuery] = useState('');

  const sections = useMemo(() => {
    const localised = NAV.map((sec) => ({
      sectionLabel: t(sec.sectionKey),
      items: sec.items.map((i) => ({ ...i, label: t(i.labelKey) })),
    }));
    if (!query.trim()) return localised;
    const q = query.toLowerCase();
    return localised
      .map((sec) => ({ ...sec, items: sec.items.filter((i) => i.label.toLowerCase().includes(q)) }))
      .filter((sec) => sec.items.length > 0);
  }, [query, t]);

  return (
    <aside
      className="sidebar"
      aria-label={t('sidebar.aria')}
      data-open={open ? 'true' : 'false'}
      aria-hidden={open ? undefined : 'true'}
    >
      <div className="sidebar__drag" />

      <div className="sidebar__search">
        <Search size={12} className="sidebar__search-icon" />
        <input
          type="search"
          className="sidebar__search-input"
          placeholder={t('sidebar.search')}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label={t('sidebar.search')}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          name="driveby-sidebar-search"
        />
      </div>

      <nav className="sidebar__nav">
        {sections.map((sec) => (
          <div key={sec.sectionLabel} className="sidebar__group">
            <div className="sidebar__group-title">{sec.sectionLabel}</div>
            {sec.items.map((item) => {
              const Icon = item.icon;
              const active = view === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`sidebar__item ${active ? 'sidebar__item--active' : ''}`}
                  onClick={() => setView(item.id)}
                  aria-current={active ? 'page' : undefined}
                >
                  <Icon size={14} className="sidebar__item-icon" />
                  <span>{item.label}</span>
                </button>
              );
            })}
          </div>
        ))}
      </nav>

      <div className="sidebar__footer">
        <div className="sidebar__brand">
          <HardDrive size={16} />
          <div className="sidebar__brand-text">
            <div className="sidebar__brand-name">driveby</div>
            <div className="sidebar__brand-version">{t('sidebar.brand.version', { version: __APP_VERSION__ })}</div>
          </div>
        </div>
      </div>
    </aside>
  );
}
