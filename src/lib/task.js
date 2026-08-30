// The task shape, and the one-way migration into it.
//
// A task used to carry a single `destination` string. From 1.7.2 it carries
// a `destinations` array, and the rules for reading it have to match
// `Task::destinations()` in src-tauri/src/backup.rs exactly — both sides
// read the same tasks.json, and the scheduler can tick before the frontend
// has rewritten it.

/// The destinations a task writes to, in the order the user listed them,
/// blanks dropped and exact repeats collapsed.
///
/// Only exact repeats: two spellings of one folder (a trailing separator, a
/// different case) are left in, because deciding they are the same folder
/// means asking the filesystem — which is the backend's job, and it refuses
/// the run when they overlap.
export function taskDestinations(task) {
  const listed = Array.isArray(task?.destinations) && task.destinations.length > 0
    ? task.destinations
    : [task?.destination];
  const seen = new Set();
  return listed
    .filter((d) => typeof d === 'string')
    .map((d) => d.trim())
    .filter((d) => d && !seen.has(d) && seen.add(d) !== false);
}

/// True if this task is still in the pre-1.7.2 shape.
function needsMigration(task) {
  return !!task
    && typeof task === 'object'
    && ('destination' in task || !Array.isArray(task.destinations));
}

/// Rewrite a stored task list into the 1.7.2 shape, dropping the legacy
/// `destination` key so nothing reads it by accident afterwards.
///
/// Returns the input array untouched when there was nothing to migrate:
/// the caller uses that identity to decide whether tasks.json needs
/// rewriting, so a normal launch doesn't produce a pointless disk write.
export function migrateTasks(tasks) {
  if (!Array.isArray(tasks)) return [];
  if (!tasks.some(needsMigration)) return tasks;
  return tasks.map((task) => {
    if (!needsMigration(task)) return task;
    const { destination: _legacy, ...rest } = task;
    return { ...rest, destinations: taskDestinations(task) };
  });
}

// ─────────────────────────────────────────────────────────────────────
// Overlap checking, for the form
// ─────────────────────────────────────────────────────────────────────
//
// The backend refuses a run whose destinations are nested in one another,
// because each destination is mirror-pruned against the source and would
// therefore delete the one nested inside it. That check canonicalises paths
// and is the one that counts. This is its lexical cousin, and it exists so
// the form can refuse a folder the moment it is picked rather than at the
// first run.

/// Whether the platform's filesystem treats `Backup` and `backup` as one
/// folder. Windows and macOS do; Linux does not. Unknown platforms fold,
/// because a false "these overlap" costs a re-pick while a false "these are
/// different" costs a deleted backup.
const FOLDS_CASE = typeof navigator === 'undefined'
  || /Windows|Macintosh|iPhone|iPad/.test(navigator.userAgent || '');

function normalizePath(p, fold) {
  const s = String(p ?? '').replace(/[\\/]+/g, '/').replace(/\/+$/, '');
  return fold ? s.toLowerCase() : s;
}

/// True if `child` is `parent` or sits underneath it.
export function pathContains(parent, child, fold = FOLDS_CASE) {
  const p = normalizePath(parent, fold);
  const c = normalizePath(child, fold);
  if (!p || !c) return false;
  return c === p || c.startsWith(`${p}/`);
}

/// The first pair of paths in `paths` where one contains the other, or null
/// when they are all disjoint.
export function findOverlap(paths, fold = FOLDS_CASE) {
  for (let i = 0; i < paths.length; i += 1) {
    for (let j = i + 1; j < paths.length; j += 1) {
      if (pathContains(paths[i], paths[j], fold) || pathContains(paths[j], paths[i], fold)) {
        return [paths[i], paths[j]];
      }
    }
  }
  return null;
}

/// The first *other task* these destinations collide with, or null.
///
/// Mirrors `reject_foreign_overlaps` in src-tauri/src/backup.rs. Each
/// destination is mirror-pruned against its own task's source, so sharing a
/// folder with another task is not sharing at all: each run deletes what the
/// other just wrote and reports it as a successful clean-up. Nesting is the
/// same thing, and another task's *source* under this destination is worse —
/// the prune would empty the folder they back up from.
///
/// Returns `{ name, path, kind }`, where `kind` is `'destination'` or
/// `'source'`, so the message can say which of the two it hit.
export function findForeignOverlap(destinations, otherTasks, fold = FOLDS_CASE) {
  for (const other of otherTasks ?? []) {
    for (const mine of destinations) {
      for (const theirs of taskDestinations(other)) {
        if (pathContains(mine, theirs, fold) || pathContains(theirs, mine, fold)) {
          return { name: other?.name ?? '', path: theirs, kind: 'destination' };
        }
      }
      const theirSource = other?.source;
      if (theirSource && pathContains(mine, theirSource, fold)) {
        return { name: other?.name ?? '', path: theirSource, kind: 'source' };
      }
    }
  }
  return null;
}
