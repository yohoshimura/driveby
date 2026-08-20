import { describe, expect, test } from 'vitest';
import {
  cutoffFor,
  trimHistory,
  DEFAULT_HISTORY_RETENTION,
  HISTORY_HARD_CAP,
} from '../history';

// Local-time constructors on both sides, so the calendar assertions hold
// whatever timezone the test machine runs in.
const local = (...parts) => new Date(...parts).getTime();

// History is stored newest-first; these helpers build it that way.
const at = (iso) => ({ id: iso, timestamp: iso });
const rows = (n, iso = new Date().toISOString()) =>
  Array.from({ length: n }, (_, i) => ({ id: `row-${i}`, timestamp: iso }));

describe('cutoffFor', () => {
  test('1d and 1w are fixed durations', () => {
    const now = local(2026, 2, 31, 12);
    expect(cutoffFor('1d', now)).toBe(now - 24 * 3600 * 1000);
    expect(cutoffFor('1w', now)).toBe(now - 7 * 24 * 3600 * 1000);
  });

  test('1m walks back a calendar month, clamping the day of month', () => {
    // 31 March has no counterpart in February: it must land on the 28th,
    // not spill forward into March the way a 30-day subtraction would.
    expect(cutoffFor('1m', local(2026, 2, 31, 12))).toBe(local(2026, 1, 28, 12));
    expect(cutoffFor('1m', local(2026, 6, 15, 9, 30))).toBe(local(2026, 5, 15, 9, 30));
    // Across a year boundary.
    expect(cutoffFor('1m', local(2026, 0, 10, 8))).toBe(local(2025, 11, 10, 8));
  });

  test('1y walks back a calendar year, clamping 29 February', () => {
    expect(cutoffFor('1y', local(2026, 4, 20, 12))).toBe(local(2025, 4, 20, 12));
    expect(cutoffFor('1y', local(2028, 1, 29, 12))).toBe(local(2027, 1, 28, 12));
  });

  test('all has no date cutoff', () => {
    expect(cutoffFor('all', Date.now())).toBe(null);
  });

  test('an unknown retention falls back to the default', () => {
    const now = local(2026, 2, 31, 12);
    for (const bad of [undefined, null, '', 'lots', 42, 1000]) {
      expect(cutoffFor(bad, now)).toBe(cutoffFor(DEFAULT_HISTORY_RETENTION, now));
    }
  });
});

describe('trimHistory — date retention', () => {
  const now = local(2026, 2, 31, 12);

  test('drops entries older than the window and keeps the rest', () => {
    const list = [
      at(new Date(now - 3600 * 1000).toISOString()),
      at(new Date(now - 20 * 3600 * 1000).toISOString()),
      at(new Date(now - 30 * 3600 * 1000).toISOString()),
    ];
    const out = trimHistory(list, '1d', now);
    expect(out.map((r) => r.id)).toEqual([list[0].id, list[1].id]);
  });

  test('a calendar month keeps what a 30-day window would have dropped', () => {
    // Cutoff is 28 Feb, so 1 March survives; a 30-day rule would cut at
    // 1 March and drop it.
    const list = [at(new Date(local(2026, 1, 28, 18)).toISOString())];
    expect(trimHistory(list, '1m', now)).toHaveLength(1);
    expect(trimHistory(list, '1w', now)).toHaveLength(0);
  });

  test('all keeps entries of any age', () => {
    const list = [at('2019-01-01T00:00:00.000Z')];
    expect(trimHistory(list, 'all', now)).toBe(list);
  });

  test('an unknown retention falls back to the default', () => {
    const old = [at(new Date(local(2025, 0, 1, 12)).toISOString())];
    for (const bad of [undefined, 'lots', 1000, -5]) {
      expect(trimHistory(old, bad, now)).toHaveLength(0);
    }
  });

  test('an entry with a missing or unparseable timestamp is kept', () => {
    // We do not delete what we cannot date; the hard cap still bounds the file.
    const list = [{ id: 'a' }, { id: 'b', timestamp: 'not a date' }, { id: 'c', timestamp: null }];
    expect(trimHistory(list, '1d', now)).toBe(list);
  });
});

describe('trimHistory — hard cap', () => {
  test('caps the list even when nothing is old enough to expire', () => {
    const out = trimHistory(rows(HISTORY_HARD_CAP + 20), '1y');
    expect(out).toHaveLength(HISTORY_HARD_CAP);
    // Newest-first, so the head is what survives.
    expect(out[0].id).toBe('row-0');
    expect(out[HISTORY_HARD_CAP - 1].id).toBe(`row-${HISTORY_HARD_CAP - 1}`);
  });

  test('the cap applies to all as well — that is the point of it', () => {
    expect(trimHistory(rows(HISTORY_HARD_CAP + 1), 'all')).toHaveLength(HISTORY_HARD_CAP);
  });
});

describe('trimHistory — invariants', () => {
  test('leaves an untouched list alone — same array, no copy', () => {
    const list = rows(5);
    expect(trimHistory(list, '1y')).toBe(list);
  });

  test('tolerates a non-array', () => {
    expect(trimHistory(null, '1m')).toEqual([]);
    expect(trimHistory(undefined, '1m')).toEqual([]);
  });
});
