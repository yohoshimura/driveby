// Locale-aware formatters, built once per language via makeFormatters()
// and consumed through the useFormat() hook. Everything here follows the
// app's selected language — never the OS locale, which is what
// `toLocaleString(undefined, …)` used to leak.

import { normalizeDays, parseTime } from './schedule';

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
  const dayMonth = new Intl.DateTimeFormat(locale, { day: '2-digit', month: '2-digit' });
  const weekdayShort = new Intl.DateTimeFormat(locale, { weekday: 'short' });
  const clock = new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' });

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

  // DD/MM/YYYY, HH:MM — fixed, in every language, and assembled by hand
  // rather than through Intl.
  //
  // Intl would put the month first in English, and a column that reads
  // 08/24 in one language and 24/08 in the other is a column you have to
  // stop and think about. The components come off the local Date, so the
  // instant shown is still the machine's own time.
  //
  // Empty string for a missing or unreadable timestamp: the caller owns the
  // "Never" label (it needs t(), which formatters deliberately don't), and
  // a row with a broken date should show nothing rather than "Invalid Date".
  const pad = (n) => String(n).padStart(2, '0');
  const formatTime = (iso) => {
    if (!iso) return '';
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return '';
    return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()}, ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };

  // dayKey is "YYYY-MM-DD". Parse the parts ourselves so the label can't
  // shift a day across timezones the way `new Date('YYYY-MM-DD')` (UTC
  // midnight) can.
  const formatDayLabel = (dayKey) => {
    const [y, m, d] = String(dayKey).split('-').map(Number);
    if (!y || !m || !d) return String(dayKey);
    return dayMonth.format(new Date(y, m - 1, d));
  };

  const formatNumber = (n) => (n == null ? '—' : integer.format(n));

  // 1 February 2026 is a Sunday, which is what turns a stored 0 = Sunday
  // index into a date Intl can name. Asserted in the tests, since every
  // day name below leans on it.
  const SUNDAY = new Date(2026, 1, 1);

  /// "Mon, Thu" for [1, 4], in the app's language.
  const formatWeekdays = (days) =>
    normalizeDays(days)
      .map((d) => weekdayShort.format(new Date(2026, 1, SUNDAY.getDate() + d)))
      .join(', ');

  /// A stored "HH:MM" as the locale writes it — which is 22:00 here and
  /// 10:00 PM in a 12-hour one. Empty string for a time that does not parse,
  /// so a half-typed schedule renders as nothing rather than as "Invalid
  /// Date".
  const formatClock = (time) => {
    const at = parseTime(time);
    return at ? clock.format(new Date(2026, 1, 1, at.hours, at.minutes)) : '';
  };

  return {
    formatBytes,
    formatDuration,
    formatTime,
    formatDayLabel,
    formatNumber,
    formatWeekdays,
    formatClock,
  };
}
