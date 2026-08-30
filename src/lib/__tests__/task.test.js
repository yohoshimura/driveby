import { describe, expect, test } from 'vitest';
import { findForeignOverlap, findOverlap, migrateTasks, pathContains, taskDestinations } from '../task';

describe('taskDestinations', () => {
  test('reads the plural field', () => {
    expect(taskDestinations({ destinations: ['D:/a', 'E:/b'] })).toEqual(['D:/a', 'E:/b']);
  });

  test('falls back to a pre-1.7.2 single destination', () => {
    expect(taskDestinations({ destination: 'D:/a' })).toEqual(['D:/a']);
  });

  test('the plural field wins when both are present', () => {
    // A downgrade-then-upgrade cycle can leave both behind. The array is
    // the newer of the two, so it is the one to trust.
    expect(taskDestinations({ destination: 'D:/old', destinations: ['E:/new'] })).toEqual(['E:/new']);
  });

  test('drops blanks and exact repeats, keeps order', () => {
    expect(taskDestinations({ destinations: ['E:/b', '  ', 'D:/a', 'E:/b', ''] }))
      .toEqual(['E:/b', 'D:/a']);
  });

  test('a task with neither field has no destinations', () => {
    expect(taskDestinations({})).toEqual([]);
    expect(taskDestinations(null)).toEqual([]);
    expect(taskDestinations({ destinations: [] })).toEqual([]);
  });
});

describe('migrateTasks', () => {
  test('rewrites a legacy task and removes the old key', () => {
    const [out] = migrateTasks([{ id: '1', destination: 'D:/a' }]);
    expect(out).toEqual({ id: '1', destinations: ['D:/a'] });
    expect('destination' in out).toBe(false);
  });

  test('leaves an already-migrated list identical — same array', () => {
    // The caller keys "does tasks.json need rewriting?" off this identity.
    const tasks = [{ id: '1', destinations: ['D:/a'] }];
    expect(migrateTasks(tasks)).toBe(tasks);
  });

  test('is idempotent, including for a task with no destination at all', () => {
    const once = migrateTasks([{ id: '1' }]);
    expect(once[0]).toEqual({ id: '1', destinations: [] });
    expect(migrateTasks(once)).toBe(once);
  });

  test('migrates only the tasks that need it', () => {
    const fresh = { id: '2', destinations: ['E:/b'] };
    const out = migrateTasks([{ id: '1', destination: 'D:/a' }, fresh]);
    expect(out[1]).toBe(fresh);
  });

  test('tolerates a non-array', () => {
    expect(migrateTasks(null)).toEqual([]);
  });
});

describe('pathContains', () => {
  test('a folder contains itself', () => {
    expect(pathContains('D:/backup', 'D:/backup', true)).toBe(true);
  });

  test('a child is contained, a sibling is not', () => {
    expect(pathContains('D:/backup', 'D:/backup/docs', true)).toBe(true);
    expect(pathContains('D:/backup', 'D:/other', true)).toBe(false);
  });

  test('a name that merely starts the same is not nested', () => {
    // D:/backup must not appear to contain D:/backup-old.
    expect(pathContains('D:/backup', 'D:/backup-old', true)).toBe(false);
  });

  test('separators and trailing slashes do not change the answer', () => {
    expect(pathContains('D:\\backup\\', 'D:/backup/docs', true)).toBe(true);
  });

  test('case folding follows the platform', () => {
    expect(pathContains('D:/Backup', 'd:/backup/x', true)).toBe(true);
    expect(pathContains('/srv/Backup', '/srv/backup/x', false)).toBe(false);
  });

  test('an empty path contains nothing', () => {
    expect(pathContains('', 'D:/backup', true)).toBe(false);
    expect(pathContains('D:/backup', '', true)).toBe(false);
  });
});

describe('findOverlap', () => {
  test('reports the offending pair', () => {
    expect(findOverlap(['D:/a', 'E:/b', 'D:/a/inner'], true)).toEqual(['D:/a', 'D:/a/inner']);
  });

  test('null when every path is disjoint', () => {
    expect(findOverlap(['D:/a', 'E:/b', 'F:/c'], true)).toBeNull();
    expect(findOverlap([], true)).toBeNull();
  });

  test('catches the same folder listed twice under two spellings', () => {
    expect(findOverlap(['D:/a', 'D:\\A\\'], true)).toEqual(['D:/a', 'D:\\A\\']);
  });
});

describe('findForeignOverlap', () => {
  const task = (id, name, source, destinations) => ({ id, name, source, destinations });

  test('catches another task writing to the same folder', () => {
    const others = [task('b', 'Docs', '/docs', ['/backup'])];
    expect(findForeignOverlap(['/backup'], others, true))
      .toEqual({ name: 'Docs', path: '/backup', kind: 'destination' });
  });

  test('catches nesting in either direction', () => {
    const outer = [task('b', 'Docs', '/docs', ['/backup'])];
    expect(findForeignOverlap(['/backup/photos'], outer, true)).not.toBeNull();
    const inner = [task('b', 'Docs', '/docs', ['/backup/photos'])];
    expect(findForeignOverlap(['/backup'], inner, true)).not.toBeNull();
  });

  test('catches a destination that would swallow another task source', () => {
    const others = [task('b', 'Archive', '/mirror/2024', ['/archive'])];
    expect(findForeignOverlap(['/mirror'], others, true))
      .toEqual({ name: 'Archive', path: '/mirror/2024', kind: 'source' });
  });

  test('allows separate folders on one drive', () => {
    const others = [task('b', 'Docs', '/docs', ['/backup/docs'])];
    expect(findForeignOverlap(['/backup/photos'], others, true)).toBeNull();
  });

  test('reads legacy single-destination tasks too', () => {
    const legacy = [{ id: 'b', name: 'Old', source: '/docs', destination: '/backup' }];
    expect(findForeignOverlap(['/backup'], legacy, true)?.name).toBe('Old');
  });

  test('is null with nothing to compare against', () => {
    expect(findForeignOverlap(['/backup'], [], true)).toBeNull();
    expect(findForeignOverlap(['/backup'], undefined, true)).toBeNull();
  });
});
