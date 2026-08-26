import { describe, expect, test } from 'vitest';
import { messageKeys, SUPPORTED_LANGUAGES, translate } from '../i18n';

describe('translate plurals', () => {
  test('a count of one picks the .one form', () => {
    expect(translate('en', 'restore.toast.success', { n: 1, count: 1 })).toBe('Restored 1 file');
    expect(translate('fr', 'restore.toast.success', { n: 1, count: 1 })).toBe('1 fichier restauré');
  });

  test('other counts pick the .other form', () => {
    expect(translate('en', 'restore.toast.success', { n: 3, count: 3 })).toBe('Restored 3 files');
    expect(translate('fr', 'restore.toast.success', { n: 3, count: 3 })).toBe('3 fichiers restaurés');
  });

  test('keys without plural forms still resolve with a count present', () => {
    expect(translate('en', 'backup.toast.complete', { count: 2 })).toBe('Backup complete');
  });
});

describe('translate fallback chain', () => {
  test('locale first, then English, then the key itself', () => {
    expect(translate('fr', 'settings.theme.dark')).toBe('Sombre');
    expect(translate('fr', 'nonexistent.key')).toBe('nonexistent.key');
  });

  test('interpolates params', () => {
    expect(translate('en', 'sidebar.brand.version', { version: '1.6.0' })).toBe('Version 1.6.0');
  });
});

describe('locale parity', () => {
  // Every string the UI can reach has to exist in both locales. Without
  // this, a key added to `en` alone renders as English inside a French
  // window: translate() falls back key by key, so nothing ever fails and
  // the gap is only ever noticed by a reader.
  const en = new Set(messageKeys('en'));
  const fr = new Set(messageKeys('fr'));

  test('every English key has a French translation', () => {
    expect([...en].filter((k) => !fr.has(k))).toEqual([]);
  });

  test('no French key is left over from a removed English one', () => {
    expect([...fr].filter((k) => !en.has(k))).toEqual([]);
  });

  test('every supported language carries messages', () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      expect(messageKeys(lang).length).toBeGreaterThan(0);
    }
  });
});
