import React from 'react';
import { useApp } from '../context/AppContext';
import Button from './common/Button';
import Toggle from './common/Toggle';
import InfoTip from './common/InfoTip';
import { bridge } from '../lib/tauri';
import { useT } from '../hooks/useT';
import { SUPPORTED_LANGUAGES, LANGUAGE_LABELS, DEFAULT_LANGUAGE } from '../lib/i18n';

const THEME_KEYS = {
  light: 'settings.theme.light',
  dark: 'settings.theme.dark',
  system: 'settings.theme.system',
};

export default function Settings() {
  const { settings, updateSetting, showToast } = useApp();
  const t = useT();

  const pickDefaultDestination = async () => {
    const p = await bridge.selectDirectory(t('settings.dialog.default_dest'));
    if (p) updateSetting('defaultDestination', p);
  };

  const revealLogs = async () => {
    try {
      const dir = await bridge.revealLogsFolder();
      await bridge.revealFolder(dir);
    } catch (e) {
      showToast(t('settings.toast.cannot_open_logs', { error: e }), 'error');
    }
  };

  const activeLang = SUPPORTED_LANGUAGES.includes(settings.language)
    ? settings.language
    : DEFAULT_LANGUAGE;

  return (
    <>
      <div className="group-title">{t('settings.section.general')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.default_dest')}</div>
          </div>
          <div className="setting-row__control" style={{ maxWidth: 380 }}>
            <div className="field-row" style={{ maxWidth: 360 }}>
              <input
                type="text"
                value={settings.defaultDestination}
                readOnly
                placeholder={t('common.notset')}
                className="field field--readonly"
                style={{ minWidth: 180 }}
                aria-label={t('settings.label.default_dest')}
                autoComplete="off"
                name="driveby-default-destination"
              />
              <Button size="small" onClick={pickDefaultDestination}>{t('common.choose')}</Button>
              {settings.defaultDestination && (
                <Button size="small" variant="borderless" destructive onClick={() => updateSetting('defaultDestination', '')}>
                  {t('common.clear')}
                </Button>
              )}
            </div>
          </div>
        </div>

        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.confirm_backup')}</div>
          </div>
          <div className="setting-row__control">
            <Toggle
              value={settings.confirmBeforeBackup}
              onChange={(v) => updateSetting('confirmBeforeBackup', v)}
              label={t('settings.label.confirm_backup')}
            />
          </div>
        </div>

        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.notifications')}</div>
          </div>
          <div className="setting-row__control">
            <Toggle
              value={settings.showNotifications}
              onChange={(v) => updateSetting('showNotifications', v)}
              label={t('settings.label.notifications')}
            />
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.section.options')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.verify')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.verify')} />
            <Toggle
              value={!!settings.verify}
              onChange={(v) => updateSetting('verify', v)}
              label={t('settings.label.verify')}
            />
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.continue_on_error')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.continue_on_error')} />
            <Toggle
              value={settings.continueOnError !== false}
              onChange={(v) => updateSetting('continueOnError', v)}
              label={t('settings.label.continue_on_error')}
            />
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.preserve_mtime')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.preserve_mtime')} />
            <Toggle
              value={settings.preserveMtime !== false}
              onChange={(v) => updateSetting('preserveMtime', v)}
              label={t('settings.label.preserve_mtime')}
            />
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.section.filtering')}</div>
      <div className="group">
        <div className="setting-row setting-row--stacked">
          <div className="setting-row__control" style={{ justifyContent: 'flex-start', justifySelf: 'start' }}>
            <div className="setting-row__label">{t('settings.label.exclude')}</div>
            <InfoTip placement="right" text={t('settings.tip.exclude')} />
          </div>
          <textarea
            value={settings.excludePatterns}
            onChange={(e) => updateSetting('excludePatterns', e.target.value)}
            placeholder={t('settings.placeholder.exclude')}
            className="field field--textarea"
            rows={5}
            aria-label={t('settings.label.exclude')}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            name="driveby-exclude-patterns"
          />
        </div>
      </div>

      <div className="group-title">{t('settings.section.appearance')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.appearance')}</div>
          </div>
          <div className="segmented" role="radiogroup" aria-label={t('settings.label.appearance')}>
            {['light', 'dark', 'system'].map((opt) => (
              <button
                key={opt}
                role="radio"
                aria-checked={settings.theme === opt}
                className={`segmented__btn ${settings.theme === opt ? 'segmented__btn--active' : ''}`}
                onClick={() => updateSetting('theme', opt)}
              >
                {t(THEME_KEYS[opt])}
              </button>
            ))}
          </div>
        </div>

      </div>

      <div className="group-title">{t('settings.section.language')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.language')}</div>
          </div>
          <div className="segmented" role="radiogroup" aria-label={t('settings.label.language')}>
            {SUPPORTED_LANGUAGES.map((code) => (
              <button
                key={code}
                role="radio"
                aria-checked={activeLang === code}
                className={`segmented__btn ${activeLang === code ? 'segmented__btn--active' : ''}`}
                onClick={() => updateSetting('language', code)}
              >
                {LANGUAGE_LABELS[code]}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.section.diagnostics')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.logs')}</div>
          </div>
          <Button size="small" onClick={revealLogs}>{t('common.open')}</Button>
        </div>
      </div>
    </>
  );
}
