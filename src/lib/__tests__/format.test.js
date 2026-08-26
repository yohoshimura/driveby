import { describe, expect, test } from 'vitest';
import { makeFormatters } from '../format';

describe('makeFormatters (fr)', () => {
  const fr = makeFormatters('fr');

  test('uses French byte units and a decimal comma', () => {
    expect(fr.formatBytes(1536)).toBe('1,50 Ko');
    expect(fr.formatBytes(0)).toBe('0 o');
  });

  test('groups thousands the French way', () => {
    // ICU uses a (narrow) no-break space as the French group separator.
    expect(fr.formatNumber(12345)).toMatch(/^12[\s  ]345$/u);
  });

  test('day label follows the locale (DD/MM)', () => {
    expect(fr.formatDayLabel('2026-08-19')).toBe('19/08');
  });

  test('dates follow the app language, not the OS locale', () => {
    const s = fr.formatTime('2026-08-19T14:05:00Z');
    expect(s).toContain('août');
  });
});

describe('makeFormatters (en)', () => {
  const en = makeFormatters('en');

  test('formats bytes with a decimal point', () => {
    expect(en.formatBytes(1536)).toBe('1.50 KB');
  });

  test('day label is MM/DD in English', () => {
    expect(en.formatDayLabel('2026-08-19')).toBe('08/19');
  });

  test('placeholder dash for an absent duration', () => {
    expect(en.formatDuration(null)).toBe('—');
  });

  test('durations keep their compact shape', () => {
    expect(en.formatDuration(42)).toBe('42s');
    expect(en.formatDuration(150)).toBe('2m 30s');
    expect(en.formatDuration(3720)).toBe('1h 2m');
  });

  test('formatTime is empty for a missing timestamp — callers own the label', () => {
    expect(en.formatTime(null)).toBe('');
  });
});

describe('weekday and clock formatting', () => {
  test('1 February 2026 is a Sunday, which the day names are anchored on', () => {
    expect(new Date(2026, 1, 1).getDay()).toBe(0);
  });

  test('names the days in order, whatever order they were given in', () => {
    const { formatWeekdays } = makeFormatters('en');
    expect(formatWeekdays([4, 1])).toBe('Mon, Thu');
    expect(formatWeekdays([0])).toBe('Sun');
    expect(formatWeekdays([])).toBe('');
  });

  test('follows the app language, not the OS locale', () => {
    const { formatWeekdays } = makeFormatters('fr');
    expect(formatWeekdays([1]).toLowerCase()).toContain('lun');
  });

  test('renders a stored time, and nothing at all for a broken one', () => {
    const { formatClock } = makeFormatters('fr');
    expect(formatClock('22:00')).toBe('22:00');
    expect(formatClock('nope')).toBe('');
  });
});
