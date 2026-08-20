import { useMemo } from 'react';
import { useApp } from '../context/AppContext';
import { makeFormatters } from '../lib/format';
import { DEFAULT_LANGUAGE, SUPPORTED_LANGUAGES } from '../lib/i18n';

// Locale-aware formatters bound to the app's selected language. Rebuilt
// only when the language changes — the Intl objects inside are cached for
// everything else.
export function useFormat() {
  const { settings } = useApp();
  const lang = SUPPORTED_LANGUAGES.includes(settings.language)
    ? settings.language
    : DEFAULT_LANGUAGE;
  return useMemo(() => makeFormatters(lang), [lang]);
}
