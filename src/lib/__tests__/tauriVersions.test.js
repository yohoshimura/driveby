import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, test } from 'vitest';

// `tauri build` refuses to run when the `tauri` crate and `@tauri-apps/api`
// are on different major.minor releases, and only the release workflow runs
// `tauri build`. Dependabot updates the two through separate ecosystems, each
// with its own cooldown, so they drift apart: 2.12 on the Rust side against
// 2.11 in npm would have passed every CI check and then failed the next
// release. This compares the two lockfiles, so CI catches the drift first.

// Both paths are fixed, relative to this file.
/* eslint-disable security/detect-non-literal-fs-filename */
const cargoLock = readFileSync(fileURLToPath(new URL('../../../src-tauri/Cargo.lock', import.meta.url)), 'utf8');
const packageLock = readFileSync(fileURLToPath(new URL('../../../package-lock.json', import.meta.url)), 'utf8');
/* eslint-enable security/detect-non-literal-fs-filename */
const majorMinor = (version) => version.split('.').slice(0, 2).join('.');

describe('Tauri versions', () => {
  test('the tauri crate and @tauri-apps/api are on the same major.minor', () => {
    const crate = cargoLock.match(/\[\[package\]\]\r?\nname = "tauri"\r?\nversion = "([^"]+)"/)?.[1];
    const npm = JSON.parse(packageLock).packages['node_modules/@tauri-apps/api']?.version;
    expect(crate, 'tauri in src-tauri/Cargo.lock').toBeTruthy();
    expect(npm, '@tauri-apps/api in package-lock.json').toBeTruthy();
    expect(majorMinor(npm), `tauri ${crate} needs @tauri-apps/api ${majorMinor(crate)}.x`).toBe(majorMinor(crate));
  });
});
