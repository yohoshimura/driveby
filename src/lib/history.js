// History is a plain JSON array rewritten in full on every change, so it
// needs a ceiling: an unbounded list makes every completed run re-serialize
// and fsync a file that only grows. The frontend is history's only writer,
// so the cap is enforced here rather than in Rust.

export const DEFAULT_HISTORY_LIMIT = 1000;

/// `limit` of 0 means unlimited. Anything invalid falls back to the default
/// rather than silently disabling the cap.
export function trimHistory(list, limit) {
  if (!Array.isArray(list)) return [];
  const cap = limit === 0 ? 0 : Number.isInteger(limit) && limit > 0 ? limit : DEFAULT_HISTORY_LIMIT;
  if (cap === 0 || list.length <= cap) return list;
  // Newest-first, so the head is what we keep.
  return list.slice(0, cap);
}
