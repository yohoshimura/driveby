import { describe, expect, test } from 'vitest';
import { findOverlap, migrateTasks, pathContains, taskDestinations } from '../task';

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
