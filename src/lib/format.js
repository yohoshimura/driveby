// Locale-aware formatters, built once per language via makeFormatters()
// and consumed through the useFormat() hook. Everything here follows the
// app's selected language — never the OS locale, which is what
// `toLocaleString(undefined, …)` used to leak.

const BYTE_UNITS = {
  en: ['B', 'KB', 'MB', 'GB', 'TB'],
  // French storage sizes are octets.
  fr: ['o', 'Ko', 'Mo', 'Go', 'To'],
};

export function makeFormatters(lang) {
  const locale = BYTE_UNITS[lang] ? lang : 'en';
  const units = BYTE_UNITS[locale];
  const integer = new Intl.NumberFormat(locale);
  const oneDecimal = new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
  const twoDecimals = new Intl.NumberFormat(locale, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  const dateTime = new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' });
  const dayMonth = new Intl.DateTimeFormat(locale, { day: '2-digit', month: '2-digit' });

  const formatBytes = (bytes) => {
    if (!bytes || bytes < 0) return `0 ${units[0]}`;
    let i = 0;
    let n = bytes;
    while (n >= 1024 && i < units.length - 1) {
      n /= 1024;
      i++;
    }
    const nf = n < 10 && i > 0 ? twoDecimals : oneDecimal;
    return `${nf.format(n)} ${units[i]}`;
  };

  const formatDuration = (seconds) => {
    if (seconds == null) return '—';
    if (seconds < 60) return `${integer.format(seconds)}s`;
    if (seconds < 3600) {
      return `${integer.format(Math.floor(seconds / 60))}m ${integer.format(seconds % 60)}s`;
    }
    return `${integer.format(Math.floor(seconds / 3600))}h ${integer.format(Math.floor((seconds % 3600) / 60))}m`;
  };

  // Empty string for a missing timestamp: the caller owns the "Never"
  // label (it needs t(), which formatters deliberately don't).
  const formatTime = (iso) => (iso ? dateTime.format(new Date(iso)) : '');

  // dayKey is "YYYY-MM-DD". Parse the parts ourselves so the label can't
  // shift a day across timezones the way `new Date('YYYY-MM-DD')` (UTC
  // midnight) can.
  const formatDayLabel = (dayKey) => {
    const [y, m, d] = String(dayKey).split('-').map(Number);
    if (!y || !m || !d) return String(dayKey);
    return dayMonth.format(new Date(y, m - 1, d));
  };

  const formatNumber = (n) => (n == null ? '—' : integer.format(n));

  return { formatBytes, formatDuration, formatTime, formatDayLabel, formatNumber };
}
