import { describe, expect, test } from 'vitest';
import { lacksRoom } from '../space';

describe('lacksRoom', () => {
  test('a destination needing more than its volume has free lacks room', () => {
    expect(lacksRoom({ reachable: true, requiredBytes: 300, availableBytes: 65 })).toBe(true);
  });

  test('exactly enough is enough, as the run counts it', () => {
    expect(lacksRoom({ reachable: true, requiredBytes: 65, availableBytes: 65 })).toBe(false);
  });

  test('a volume that would not say its free space is not judged', () => {
    // The run does not check such a volume either, so the dialog must not
    // block what the run would go ahead with.
    expect(lacksRoom({ reachable: true, requiredBytes: 300, availableBytes: null })).toBe(false);
    expect(lacksRoom({ reachable: true, requiredBytes: 300 })).toBe(false);
  });

  test('an unplugged destination has no figures to judge', () => {
    expect(lacksRoom({ reachable: false, requiredBytes: 0, availableBytes: 0 })).toBe(false);
  });
});
