import { describe, expect, test } from 'vitest';
import { trimHistory, DEFAULT_HISTORY_LIMIT } from '../history';

const rows = (n) => Array.from({ length: n }, (_, i) => ({ id: `row-${i}` }));

describe('trimHistory', () => {
  test('keeps the newest entries up to the cap', () => {
    const out = trimHistory(rows(120), 100);
    expect(out).toHaveLength(100);
    // History is stored newest-first, so the head must survive.
    expect(out[0].id).toBe('row-0');
    expect(out[99].id).toBe('row-99');
  });

  test('leaves a short list untouched — same array, no copy', () => {
    const list = rows(5);
    expect(trimHistory(list, 100)).toBe(list);
  });

  test('0 means unlimited', () => {
    const list = rows(5000);
    expect(trimHistory(list, 0)).toBe(list);
  });

  test('falls back to the default cap for missing or invalid limits', () => {
    expect(trimHistory(rows(DEFAULT_HISTORY_LIMIT + 10), undefined)).toHaveLength(
      DEFAULT_HISTORY_LIMIT,
    );
    expect(trimHistory(rows(DEFAULT_HISTORY_LIMIT + 10), 'lots')).toHaveLength(
      DEFAULT_HISTORY_LIMIT,
    );
    expect(trimHistory(rows(DEFAULT_HISTORY_LIMIT + 10), -5)).toHaveLength(DEFAULT_HISTORY_LIMIT);
  });

  test('tolerates a non-array', () => {
    expect(trimHistory(null, 100)).toEqual([]);
    expect(trimHistory(undefined, 100)).toEqual([]);
  });
});
