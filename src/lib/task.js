// The task shape, and the one-way migration into it.
//
// A task used to carry a single `destination` string. From 1.7.2 it carries
// a `destinations` array, and the rules for reading it have to match
// `Task::destinations()` in src-tauri/src/backup.rs exactly — both sides
// read the same tasks.json, and the scheduler can tick before the frontend
// has rewritten it. Sources went the same way after 1.7.4: one `source`
// string became a `sources` array of `{ path, folder }`, read by the rules
// of `Task::sources()`.

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

/// The sources a task reads, in the order the user listed them, blank paths
/// dropped and exact repeats collapsed.
///
/// A blank *folder* is kept, as `Task::sources()` keeps it: that is a source
/// nobody has named yet, and it is refused by its path rather than quietly
/// left out of a backup that then reports success.
export function taskSources(task) {
  const listed = Array.isArray(task?.sources) && task.sources.length > 0
    ? task.sources
    : legacySources(task);
  const seen = new Set();
  return listed
    .filter((s) => s && typeof s.path === 'string')
    .map((s) => ({ path: s.path.trim(), folder: typeof s.folder === 'string' ? s.folder.trim() : '' }))
    .filter((s) => s.path && !seen.has(s.path) && seen.add(s.path) !== false);
}

/// A task from before sources had folders: its one `source`, named after
/// the folder it is.
function legacySources(task) {
  const path = typeof task?.source === 'string' ? task.source.trim() : '';
  return path ? [{ path, folder: sourceFolderName(path) }] : [];
}

/// The destination folder a source is named after until the user says
/// otherwise: the last component of its path, or '' for a path with none.
///
/// The Rust side asks `Path::file_name()`, which parses by the rules of the
/// OS it is built for — `D:` is a drive on Windows and a folder name
/// everywhere else. JS cannot ask which rules apply, so this recognises the
/// roots a folder dialog can hand back on any platform: a bare drive, a
/// network share (`\\nas\photos`), and `/`. No dialog produces a path on
/// which the two answers differ.
export function sourceFolderName(path) {
  const trimmed = String(path ?? '').trim().replace(/[\\/]+$/, '');
  if (!trimmed || /^[A-Za-z]:$/.test(trimmed) || /^[\\/]{2}[^\\/]+[\\/][^\\/]+$/.test(trimmed)) {
    return '';
  }
  return trimmed.split(/[\\/]/).pop();
}

/// Why this is not a usable destination folder name, or null when it is.
///
/// The rules of `validate_folder_name` in src-tauri/src/backup.rs. A name,
/// not a path: a separator would invent a hierarchy nobody asked for, and
/// `..` would climb out of the destination altogether. The refused
/// characters are the ones Windows rejects, checked everywhere so a task
/// written on one platform does not fail only once it reaches another.
export function folderNameError(name) {
  const n = String(name ?? '').trim();
  if (!n) return 'empty';
  if (n === '.' || n === '..') return 'dots';
  if (/[\\/]/.test(n)) return 'separator';
  if (/[<>:"|?*\x00-\x1f]/.test(n)) return 'character';
  return null;
}

/// True if this task is still in a shape from before 1.7.2, or from before
/// sources had folders.
function needsMigration(task) {
  return !!task
    && typeof task === 'object'
    && ('destination' in task
      || !Array.isArray(task.destinations)
      || 'source' in task
      || !Array.isArray(task.sources));
}

/// Rewrite a stored task list into the current shape, dropping the legacy
/// `destination` and `source` keys so nothing reads them by accident
/// afterwards.
///
/// Returns the input array untouched when there was nothing to migrate:
/// the caller uses that identity to decide whether tasks.json needs
/// rewriting, so a normal launch doesn't produce a pointless disk write.
export function migrateTasks(tasks) {
  if (!Array.isArray(tasks)) return [];
  if (!tasks.some(needsMigration)) return tasks;
  return tasks.map((task) => {
    if (!needsMigration(task)) return task;
    const { destination: _legacyDestination, source: _legacySource, ...rest } = task;
    return { ...rest, destinations: taskDestinations(task), sources: taskSources(task) };
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

/// Whether each source writes into a folder of its own at the destination.
///
/// The rule of `uses_subfolders` in src-tauri/src/backup.rs: only several
/// sources need keeping apart, so a single one is mirrored straight into the
/// destination, as every version before multi-source mirrored it — and its
/// folder name is neither shown nor checked.
export function usesSubfolders(sources) {
  return (sources?.length ?? 0) > 1;
}

/// The first folder name two sources share, or null.
///
/// Compared as `preflight_sources` compares them: folded where the
/// filesystem folds, since `Photos` and `photos` are one folder there and the
/// two sources would write into each other. An unnamed source is
/// `folderNameError`'s to report — two of them are two missing names, not a
/// collision.
export function findDuplicateFolder(sources, fold = FOLDS_CASE) {
  const seen = new Set();
  for (const source of sources ?? []) {
    const name = String(source?.folder ?? '').trim();
    if (!name) continue;
    const key = fold ? name.toLowerCase() : name;
    if (seen.has(key)) return name;
    seen.add(key);
  }
  return null;
}

/// The first *other task* these destinations collide with, or null.
///
/// Mirrors `reject_foreign_overlaps` in src-tauri/src/backup.rs. Each
/// destination is mirror-pruned against its own task's sources, so sharing a
/// folder with another task is not sharing at all: each run deletes what the
/// other just wrote and reports it as a successful clean-up. Nesting is the
/// same thing, and any of another task's *sources* under this destination is
/// worse — the prune would empty the folder they back up from.
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
      for (const their of taskSources(other)) {
        if (pathContains(mine, their.path, fold)) {
          return { name: other?.name ?? '', path: their.path, kind: 'source' };
        }
      }
    }
  }
  return null;
}
