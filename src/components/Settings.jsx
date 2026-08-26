import React, { useEffect, useState } from 'react';
import { useApp } from '../context/AppContext';
import Button from './common/Button';
import Toggle from './common/Toggle';
import InfoTip from './common/InfoTip';
import { bridge } from '../lib/tauri';
import { checkForUpdate, installUpdate } from '../lib/updater';
import { useT } from '../hooks/useT';
import { HISTORY_RETENTIONS, DEFAULT_HISTORY_RETENTION } from '../lib/history';
import { SUPPORTED_LANGUAGES, LANGUAGE_LABELS, DEFAULT_LANGUAGE } from '../lib/i18n';

const THEME_KEYS = {
  light: 'settings.theme.light',
  dark: 'settings.theme.dark',
  system: 'settings.theme.system',
};

// The picker offers an age, not a count. 'all' still passes through the
// hard entry cap inside trimHistory — it means 'no date limit', not
// 'unbounded file'.
const RETENTION_KEYS = {
  '1d': 'settings.retention.1d',
  '1w': 'settings.retention.1w',
  '1m': 'settings.retention.1m',
  '1y': 'settings.retention.1y',
  all: 'common.unlimited',
};

export default function Settings() {
  const { settings, updateSetting, showToast } = useApp();
  const t = useT();
  const [autostartOn, setAutostartOn] = useState(false);
  const [updateState, setUpdateState] = useState({ status: 'idle' });
  // Kept locally while it is being typed: "50" passes through "5", and
  // committing every keystroke would leave the ceiling at 5 MB/s for anyone
  // who clicked away mid-number.
  const [maxSpeed, setMaxSpeed] = useState(() =>
    settings.maxSpeedMbps > 0 ? String(settings.maxSpeedMbps) : '');

  // The OS owns the autostart registration, so the toggle reflects what is
  // actually registered rather than what settings.json remembers.
  useEffect(() => {
    let alive = true;
    bridge.isAutostartEnabled().then((on) => {
      if (alive) setAutostartOn(on);
    });
    return () => { alive = false; };
  }, []);

  const toggleAutostart = async (want) => {
    const ok = await bridge.setAutostart(want);
    if (!ok) {
      showToast(t('settings.toast.autostart_failed'), 'error');
      return;
    }
    setAutostartOn(want);
    updateSetting('autostart', want);
  };

  const runUpdateCheck = async () => {
    setUpdateState({ status: 'checking' });
    const res = await checkForUpdate();
    if (res.available) {
      setUpdateState({ status: 'available', version: res.version, update: res.update });
    } else {
      setUpdateState({ status: 'current' });
      if (res.error) showToast(t('updates.toast.failed', { error: res.error }), 'error');
    }
  };

  const runInstall = async () => {
    // The control below is disabled while installing, but a stray second
    // call (keyboard repeat, a re-render mid-await) must not start a second
    // download and a second relaunch.
    if (updateState.status === 'installing') return;
    setUpdateState((prev) => ({ ...prev, status: 'installing' }));
    try {
      await installUpdate(updateState.update);
    } catch (e) {
      setUpdateState({ status: 'idle' });
      showToast(t('updates.toast.failed', { error: e }), 'error');
    }
  };

  /// Anything that is not a positive number means "no ceiling" — an empty
  /// field, a stray minus sign, a comma from a French keyboard that the
  /// number input rejected. The upper bound is there so a typo of 500000
  /// cannot be mistaken for a working setting.
  const commitMaxSpeed = () => {
    const parsed = Number.parseFloat(String(maxSpeed).replace(',', '.'));
    const value = Number.isFinite(parsed) && parsed > 0 ? Math.min(parsed, 10000) : 0;
    setMaxSpeed(value > 0 ? String(value) : '');
    if (value !== settings.maxSpeedMbps) updateSetting('maxSpeedMbps', value);
  };

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
  const activeParallel = [1, 2, 4, 8].includes(settings.parallelCopies)
    ? settings.parallelCopies
    : 4;
  const activeRetention = HISTORY_RETENTIONS.includes(settings.historyRetention)
    ? settings.historyRetention
    : DEFAULT_HISTORY_RETENTION;

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
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.parallel_copies')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.parallel_copies')} />
            <div className="segmented" role="radiogroup" aria-label={t('settings.label.parallel_copies')}>
              {[1, 2, 4, 8].map((n) => (
                <button
                  key={n}
                  role="radio"
                  aria-checked={activeParallel === n}
                  className={`segmented__btn ${activeParallel === n ? 'segmented__btn--active' : ''}`}
                  onClick={() => updateSetting('parallelCopies', n)}
                >
                  {n}
                </button>
              ))}
            </div>
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.max_speed')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.max_speed')} />
            <div className="field-row">
              <input
                type="number"
                min="0"
                step="1"
                inputMode="decimal"
                className="field field--compact"
                style={{ width: 90 }}
                value={maxSpeed}
                onChange={(e) => setMaxSpeed(e.target.value)}
                onBlur={commitMaxSpeed}
                onKeyDown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); }}
                placeholder={t('common.unlimited')}
                aria-label={t('settings.label.max_speed')}
                autoComplete="off"
                name="driveby-max-speed"
              />
              <span className="setting-row__sub">{t('settings.unit.mbps')}</span>
            </div>
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.history_retention')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.history_retention')} />
            <div className="segmented" role="radiogroup" aria-label={t('settings.label.history_retention')}>
              {HISTORY_RETENTIONS.map((key) => (
                <button
                  key={key}
                  role="radio"
                  aria-checked={activeRetention === key}
                  className={`segmented__btn ${activeRetention === key ? 'segmented__btn--active' : ''}`}
                  onClick={() => updateSetting('historyRetention', key)}
                >
                  {t(RETENTION_KEYS[key])}
                </button>
              ))}
            </div>
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

      <div className="group-title">{t('settings.section.background')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.close_to_tray')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.close_to_tray')} />
            <Toggle
              value={!!settings.closeToTray}
              onChange={(v) => updateSetting('closeToTray', v)}
              label={t('settings.label.close_to_tray')}
            />
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.autostart')}</div>
          </div>
          <div className="setting-row__control">
            <InfoTip text={t('settings.tip.autostart')} />
            <Toggle
              value={autostartOn}
              onChange={toggleAutostart}
              label={t('settings.label.autostart')}
            />
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.section.updates')}</div>
      <div className="group">
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.version')}</div>
            <div className="setting-row__sub">
              {updateState.status === 'installing'
                ? t('updates.installing')
                : updateState.status === 'available'
                  ? t('updates.available', { version: updateState.version })
                  : updateState.status === 'current'
                    ? t('updates.up_to_date')
                    : t('sidebar.brand.version', { version: __APP_VERSION__ })}
            </div>
          </div>
          <div className="setting-row__control">
            {/* `installing` had no branch of its own, so the moment the
                download started this fell through to the else and rendered
                an *enabled* "Check for updates" button with the current
                version beside it — indistinguishable from idle. The
                disabled={status === 'installing'} guard sat inside the
                `available` branch, where it could never be true (#F2). */}
            {updateState.status === 'installing' ? (
              <Button size="small" variant="primary" disabled>
                {t('updates.action.installing')}
              </Button>
            ) : updateState.status === 'available' ? (
              <Button size="small" variant="primary" onClick={runInstall}>
                {t('updates.action.install')}
              </Button>
            ) : (
              <Button
                size="small"
                onClick={runUpdateCheck}
                disabled={updateState.status === 'checking'}
              >
                {updateState.status === 'checking'
                  ? t('updates.action.checking')
                  : t('updates.action.check')}
              </Button>
            )}
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">{t('settings.label.check_updates_on_start')}</div>
          </div>
          <div className="setting-row__control">
            <Toggle
              value={settings.checkUpdatesOnStart !== false}
              onChange={(v) => updateSetting('checkUpdatesOnStart', v)}
              label={t('settings.label.check_updates_on_start')}
            />
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
