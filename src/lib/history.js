// History is a plain JSON array rewritten in full on every change, so it
// needs a ceiling: an unbounded list makes every completed run re-serialize
// and fsync a file that only grows. The frontend is history's only writer,
// so the trimming is enforced here rather than in Rust.
//
// The ceiling has two halves. What the user picks is an *age* — keep the
// last day, week, month, year — because "how far back can I look" is the
// question they actually have. Behind it sits a hard entry cap that no
// setting can lift, including "All": an hourly task left running for a
// year is 8 760 rows, and nothing in the UI reads that far back.

export const HISTORY_RETENTIONS = ['1d', '1w', '1m', '1y', 'all'];
export const DEFAULT_HISTORY_RETENTION = '1m';
export const HISTORY_HARD_CAP = 5000;

const DAY_MS = 24 * 60 * 60 * 1000;

/// Walks `date` back by `months` calendar months, clamping the day of the
/// month to the target month's length — 31 March minus one month is 28
/// February, not 3 March. This mirrors the Rust scheduler's
/// `checked_add_months`, so "a month" means the same thing on both sides.
function shiftMonths(date, months) {
  const day = date.getDate();
  const shifted = new Date(date.getTime());
  // Park on the 1st first: setMonth on the 31st would overflow forward
  // before we ever get to clamp it.
  shifted.setDate(1);
  shifted.setMonth(shifted.getMonth() + months);
  const lastDay = new Date(shifted.getFullYear(), shifted.getMonth() + 1, 0).getDate();
  shifted.setDate(Math.min(day, lastDay));
  return shifted;
}

/// The epoch-ms instant before which entries expire, or `null` for "all"
/// (no date cutoff — only the hard cap applies). An unrecognised retention
/// falls back to the default rather than silently keeping everything: a
/// settings.json written by an older version holds a *number* here.
export function cutoffFor(retention, now = Date.now()) {
  const key = HISTORY_RETENTIONS.includes(retention) ? retention : DEFAULT_HISTORY_RETENTION;
  switch (key) {
    case 'all': return null;
    case '1d': return now - DAY_MS;
    case '1w': return now - 7 * DAY_MS;
    case '1m': return shiftMonths(new Date(now), -1).getTime();
    default: return shiftMonths(new Date(now), -12).getTime();
  }
}

/// Drops entries older than `retention`, then enforces the hard cap.
/// Returns `list` itself when nothing was removed, so an unchanged history
/// doesn't trigger a re-render and a rewrite of history.json.
export function trimHistory(list, retention, now = Date.now()) {
  if (!Array.isArray(list)) return [];

  const cutoff = cutoffFor(retention, now);
  // An entry with no usable timestamp is kept: we don't delete what we
  // can't date. The hard cap below still bounds the file.
  const kept = cutoff === null
    ? list
    : list.filter((entry) => {
      const ts = Date.parse(entry?.timestamp);
      return Number.isNaN(ts) || ts >= cutoff;
    });

  // Newest-first, so the head is what survives the cap.
  if (kept.length > HISTORY_HARD_CAP) return kept.slice(0, HISTORY_HARD_CAP);
  return kept.length === list.length ? list : kept;
}
