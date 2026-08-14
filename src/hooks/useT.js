import { useCallback } from 'react';
import { useApp } from '../context/AppContext';
import { translate, DEFAULT_LANGUAGE, SUPPORTED_LANGUAGES } from '../lib/i18n';

// `t(key, params?)` returns the active-locale string. Falls back to English
// if a key is missing in the chosen locale, then to the key itself if it's
// missing everywhere — the UI never renders an empty cell.
export function useT() {
  const { settings } = useApp();
  const lang = SUPPORTED_LANGUAGES.includes(settings.language)
    ? settings.language
    : DEFAULT_LANGUAGE;
  return useCallback((key, params) => translate(lang, key, params), [lang]);
}
