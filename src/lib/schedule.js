// Custom schedules: "on these days, at this time", in local time.
//
// The Rust scheduler decides when a task actually runs — see
// `last_occurrence` in src-tauri/src/scheduler.rs. This is the mirror that
// lets the form say what the next run will be, and it has to agree with it
// on the two rules that matter: days are numbered 0 = Sunday, and a schedule
// with no days or an unreadable time never fires at all.
//
// The repo already keeps one such mirror — `shiftMonths` here against
// `checked_add_months` there — for the same reason: the answer is needed on
// both sides of the bridge, and a round trip to compute a label is worse
// than a rule written twice with a comment tying them together.

export const DEFAULT_SCHEDULE_TIME = '22:00';
/// 0 = Sunday, the order the day picker shows them in.
export const WEEKDAY_INDEXES = [0, 1, 2, 3, 4, 5, 6];

/// "HH:MM" -> { hours, minutes }, or null if it is not a time.
export function parseTime(value) {
  const match = /^(\d{1,2}):(\d{2})$/.exec(String(value ?? '').trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return { hours, minutes };
}

/// The days a schedule is valid for: whole numbers 0-6, no repeats, in
/// order. Anything else is dropped rather than corrected.
export function normalizeDays(days) {
  const clean = (days || []).filter((d) => Number.isInteger(d) && d >= 0 && d <= 6);
  return [...new Set(clean)].sort((a, b) => a - b);
}

/// The next moment this schedule comes round, at or after `now`, or null
/// when it never does.
export function nextOccurrence(days, time, now = new Date()) {
  const at = parseTime(time);
  const wanted = new Set(normalizeDays(days));
  if (!at || wanted.size === 0) return null;
  // Up to and including 7 days ahead: when today is the only listed day and
  // its time has passed, the answer is a week away.
  for (let ahead = 0; ahead <= 7; ahead += 1) {
    const candidate = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() + ahead,
      at.hours,
      at.minutes,
      0,
      0,
    );
    if (!wanted.has(candidate.getDay())) continue;
    if (candidate >= now) return candidate;
  }
  return null;
}
