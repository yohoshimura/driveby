import { describe, expect, test } from 'vitest';
import {
  findDuplicateFolder,
  findForeignOverlap,
  findOverlap,
  folderNameError,
  migrateTasks,
  pathContains,
  sourceFolderName,
  taskDestinations,
  taskSources,
} from '../task';

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

describe('taskSources', () => {
  test('folds a task from before sources had folders into one, named after it', () => {
    expect(taskSources({ source: 'C:/Photos' })).toEqual([{ path: 'C:/Photos', folder: 'Photos' }]);
  });

  test('the plural field wins when both are present', () => {
    const task = { source: 'C:/Ignored', sources: [{ path: 'C:/Photos', folder: 'Pics' }] };
    expect(taskSources(task)).toEqual([{ path: 'C:/Photos', folder: 'Pics' }]);
  });

  test('drops blank paths and exact repeats, keeps order', () => {
    const task = {
      sources: [
        { path: 'D:/b', folder: 'b' },
        { path: '  ', folder: 'Blank' },
        { path: 'C:/a', folder: 'a' },
        { path: 'D:/b', folder: 'b' },
      ],
    };
    expect(taskSources(task).map((s) => s.path)).toEqual(['D:/b', 'C:/a']);
  });

  test('keeps a source nobody has named, so it is refused rather than left out', () => {
    // Task::sources() keeps it too. Dropping it backed up the other sources
    // and reported success while this one was never read.
    expect(taskSources({ sources: [{ path: 'C:/a', folder: '  ' }] }))
      .toEqual([{ path: 'C:/a', folder: '' }]);
  });

  test('a drive root keeps its path and waits for a name', () => {
    expect(taskSources({ source: 'D:\\' })).toEqual([{ path: 'D:\\', folder: '' }]);
  });

  test('a task with neither field has no sources', () => {
    expect(taskSources({})).toEqual([]);
    expect(taskSources(null)).toEqual([]);
    expect(taskSources({ sources: [] })).toEqual([]);
  });
});

describe('sourceFolderName', () => {
  test('is the last component, whichever separator and however it ends', () => {
    expect(sourceFolderName('C:\\Users\\y\\Photos')).toBe('Photos');
    expect(sourceFolderName('C:/Users/y/Photos/')).toBe('Photos');
    expect(sourceFolderName('/home/y/Documents')).toBe('Documents');
    expect(sourceFolderName('\\\\nas\\photos\\2024')).toBe('2024');
  });

  test('is empty for every root a folder dialog can hand back', () => {
    // Path::file_name() answers None for each of these on its own platform.
    for (const root of ['D:\\', 'D:/', 'D:', '\\\\nas\\photos', '\\\\nas\\photos\\', '/', '', '  ']) {
      expect(sourceFolderName(root)).toBe('');
    }
  });
});

describe('folderNameError', () => {
  test('refuses anything that is not a folder name', () => {
    expect(folderNameError('')).toBe('empty');
    expect(folderNameError('   ')).toBe('empty');
    expect(folderNameError('.')).toBe('dots');
    expect(folderNameError('..')).toBe('dots');
    expect(folderNameError('a/b')).toBe('separator');
    expect(folderNameError('a\\b')).toBe('separator');
    expect(folderNameError('x<y')).toBe('character');
    expect(folderNameError('D:')).toBe('character');
    expect(folderNameError('tab\there')).toBe('character');
  });

  test('accepts ordinary folder names', () => {
    for (const good of ['Photos', 'Photos-Work', 'Mes documents', '2024.backup']) {
      expect(folderNameError(good)).toBeNull();
    }
  });
});

describe('findDuplicateFolder', () => {
  test('folds case where the filesystem does', () => {
    const sources = [{ path: 'C:/a', folder: 'Photos' }, { path: 'D:/b', folder: 'photos' }];
    expect(findDuplicateFolder(sources, true)).toBe('photos');
    expect(findDuplicateFolder(sources, false)).toBeNull();
  });

  test('two unnamed sources are two missing names, not a collision', () => {
    const sources = [{ path: 'C:/a', folder: '' }, { path: 'D:/b', folder: ' ' }];
    expect(findDuplicateFolder(sources, true)).toBeNull();
  });
});

describe('migrateTasks', () => {
  test('rewrites a legacy task and removes the old keys', () => {
    const [out] = migrateTasks([{ id: '1', source: 'C:/Photos', destination: 'D:/a' }]);
    expect(out).toEqual({
      id: '1',
      destinations: ['D:/a'],
      sources: [{ path: 'C:/Photos', folder: 'Photos' }],
    });
    expect('destination' in out).toBe(false);
    expect('source' in out).toBe(false);
  });

  test('a whole-drive source survives the rewrite, waiting for a name', () => {
    const [out] = migrateTasks([{ id: '1', source: 'D:\\', destinations: ['E:/b'] }]);
    expect(out.sources).toEqual([{ path: 'D:\\', folder: '' }]);
  });

  test('leaves an already-migrated list identical — same array', () => {
    // The caller keys "does tasks.json need rewriting?" off this identity.
    const tasks = [{ id: '1', destinations: ['D:/a'], sources: [{ path: 'C:/s', folder: 's' }] }];
    expect(migrateTasks(tasks)).toBe(tasks);
  });

  test('is idempotent, including for a task with no destination or source at all', () => {
    const once = migrateTasks([{ id: '1' }]);
    expect(once[0]).toEqual({ id: '1', destinations: [], sources: [] });
    expect(migrateTasks(once)).toBe(once);
  });

  test('migrates only the tasks that need it', () => {
    const fresh = { id: '2', destinations: ['E:/b'], sources: [{ path: 'C:/s', folder: 's' }] };
    const out = migrateTasks([{ id: '1', destination: 'D:/a' }, fresh]);
    expect(out[1]).toBe(fresh);
  });

  test('a list migrated for destinations alone still gets its sources', () => {
    // Every tasks.json written by 1.7.2 through 1.7.4 is in this shape.
    const [out] = migrateTasks([{ id: '1', source: 'C:/Photos', destinations: ['D:/a'] }]);
    expect(out.sources).toEqual([{ path: 'C:/Photos', folder: 'Photos' }]);
    expect('source' in out).toBe(false);
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

  test('checks every source of the other task, not just the first', () => {
    const other = {
      name: 'Two',
      sources: [{ path: '/a', folder: 'A' }, { path: '/b', folder: 'B' }],
      destinations: ['/elsewhere'],
    };
    expect(findForeignOverlap(['/b'], [other], true))
      .toEqual({ name: 'Two', path: '/b', kind: 'source' });
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
