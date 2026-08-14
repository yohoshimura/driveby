import { useEffect, useState } from 'react';

export function useSystemTheme(preference) {
  const getSystem = () =>
    typeof window !== 'undefined' && window.matchMedia
      ? window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
      : 'light';

  const [systemTheme, setSystemTheme] = useState(getSystem);

  useEffect(() => {
    if (!window.matchMedia) return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e) => setSystemTheme(e.matches ? 'dark' : 'light');
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  const resolved = preference === 'system' ? systemTheme : preference;
  return { resolved, systemTheme };
}
