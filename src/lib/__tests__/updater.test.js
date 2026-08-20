import { beforeEach, describe, expect, test, vi } from 'vitest';

// vi.mock is hoisted above the imports, so the spies have to be created in a
// hoisted block too or the factories would close over uninitialised bindings.
const { check, relaunch } = vi.hoisted(() => ({ check: vi.fn(), relaunch: vi.fn() }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch }));

import { checkForUpdate, installUpdate } from '../updater';

beforeEach(() => {
  check.mockReset();
  relaunch.mockReset();
});

describe('checkForUpdate', () => {
  test('reports no update when the endpoint says the app is current', async () => {
    check.mockResolvedValue(null);
    expect(await checkForUpdate()).toEqual({ available: false });
  });

  test('passes the version and notes through when an update exists', async () => {
    const update = { version: '1.6.2', body: 'See CHANGELOG.md' };
    check.mockResolvedValue(update);
    expect(await checkForUpdate()).toEqual({
      available: true,
      version: '1.6.2',
      notes: 'See CHANGELOG.md',
      update,
    });
  });

  // The three ways this can fail in production all land in the same catch, and
  // all of them look identical to "you are up to date" in the UI. These tests
  // pin that it is a deliberate swallow that still surfaces the reason, so a
  // broken updater is diagnosable from the returned object rather than silent.
  test.each([
    ['the release feed is missing (no release published yet)', 'Could not fetch a valid release JSON: 404 Not Found'],
    ['the signature does not verify against the bundled pubkey', 'Signature verification failed'],
    ['the machine is offline', 'error sending request for url'],
  ])('degrades to "no update" when %s', async (_label, message) => {
    check.mockRejectedValue(new Error(message));

    const res = await checkForUpdate();

    expect(res.available).toBe(false);
    expect(res.error).toContain(message);
  });

  test('never throws into the caller, whatever the plugin rejects with', async () => {
    check.mockRejectedValue('a bare string, not an Error');
    await expect(checkForUpdate()).resolves.toMatchObject({ available: false });
  });
});

describe('installUpdate', () => {
  test('installs first, then relaunches', async () => {
    const order = [];
    const update = {
      downloadAndInstall: vi.fn(async () => {
        order.push('install');
      }),
    };
    relaunch.mockImplementation(async () => {
      order.push('relaunch');
    });

    await installUpdate(update);

    expect(order).toEqual(['install', 'relaunch']);
  });

  test('does not relaunch when the download fails', async () => {
    const update = { downloadAndInstall: vi.fn().mockRejectedValue(new Error('disk full')) };

    await expect(installUpdate(update)).rejects.toThrow('disk full');
    expect(relaunch).not.toHaveBeenCalled();
  });
});
