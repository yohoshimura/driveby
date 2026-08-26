import { describe, expect, test } from 'vitest';
import { nextOccurrence, normalizeDays, parseTime } from '../schedule';

// Local-time constructors on both sides, so the assertions hold whatever
// timezone the test machine runs in — the same reason the Rust tests pin an
// offset instead of using `Local`.
const local = (...parts) => new Date(...parts);

// 27 August 2026 is a Thursday. Asserted rather than assumed, since every
// case below hangs on it.
const THU = local(2026, 7, 27, 12, 0);

describe('the reference date', () => {
  test('27 August 2026 is a Thursday', () => {
    expect(THU.getDay()).toBe(4);
  });
});

describe('parseTime', () => {
  test('reads a time of day', () => {
    expect(parseTime('22:00')).toEqual({ hours: 22, minutes: 0 });
    expect(parseTime('7:05')).toEqual({ hours: 7, minutes: 5 });
  });

  test('refuses anything that is not one', () => {
    expect(parseTime('24:00')).toBeNull();
    expect(parseTime('22:60')).toBeNull();
    expect(parseTime('late')).toBeNull();
    expect(parseTime('')).toBeNull();
    expect(parseTime(null)).toBeNull();
  });
});

describe('normalizeDays', () => {
  test('sorts, de-duplicates and drops what is not a weekday', () => {
    expect(normalizeDays([4, 1, 4, 9, -1, 'mon', null])).toEqual([1, 4]);
    expect(normalizeDays(null)).toEqual([]);
  });
});

describe('nextOccurrence', () => {
  test('later today when the time has not passed', () => {
    const next = nextOccurrence([1, 4], '22:00', local(2026, 7, 27, 21, 59));
    expect(next).toEqual(local(2026, 7, 27, 22, 0));
  });

  test('the next listed day once today has passed', () => {
    // Thursday 22:01, listed Monday and Thursday: next is Monday the 31st.
    const next = nextOccurrence([1, 4], '22:00', local(2026, 7, 27, 22, 1));
    expect(next).toEqual(local(2026, 7, 31, 22, 0));
  });

  test('a week away when today is the only listed day and it has passed', () => {
    const next = nextOccurrence([4], '22:00', local(2026, 7, 27, 22, 1));
    expect(next).toEqual(local(2026, 8, 3, 22, 0));
  });

  test('exactly now counts as due now, not in a week', () => {
    const next = nextOccurrence([4], '22:00', local(2026, 7, 27, 22, 0, 0));
    expect(next).toEqual(local(2026, 7, 27, 22, 0));
  });

  test('crosses a month boundary', () => {
    const next = nextOccurrence([2], '07:30', local(2026, 7, 30, 9, 0));
    expect(next).toEqual(local(2026, 8, 1, 7, 30));
  });

  test('null when the schedule can never fire', () => {
    // The same two conditions that make the Rust side treat the task as
    // inert: no days, or a time it cannot read.
    expect(nextOccurrence([], '22:00', THU)).toBeNull();
    expect(nextOccurrence(null, '22:00', THU)).toBeNull();
    expect(nextOccurrence([1], 'later', THU)).toBeNull();
    expect(nextOccurrence([1], '', THU)).toBeNull();
  });
});
