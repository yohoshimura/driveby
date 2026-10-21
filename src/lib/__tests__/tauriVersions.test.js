import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, test } from 'vitest';

// `tauri build` refuses to run when a Tauri crate and its npm package are on
// different major.minor releases — the core (`tauri` / `@tauri-apps/api`) and
// every plugin (`tauri-plugin-x` / `@tauri-apps/plugin-x`) alike — and only
// the release workflow runs `tauri build`. Dependabot updates the two sides
// through separate ecosystems, each with its own cooldown, so they drift
// apart: 2.12 on the Rust side against 2.11 in npm would have passed every CI
// check and then failed the next release. This compares the two lockfiles,
// so CI catches the drift first — and Dependabot's auto-merge, which waits
// for CI, never merges one side alone.

// Both paths are fixed, relative to this file.
/* eslint-disable security/detect-non-literal-fs-filename */
const cargoLock = readFileSync(fileURLToPath(new URL('../../../src-tauri/Cargo.lock', import.meta.url)), 'utf8');
const packageLock = readFileSync(fileURLToPath(new URL('../../../package-lock.json', import.meta.url)), 'utf8');
/* eslint-enable security/detect-non-literal-fs-filename */
const majorMinor = (version) => version.split('.').slice(0, 2).join('.');

// Every [[package]] in Cargo.lock, name -> version. A crate locked at two
// versions keeps the last one; no Tauri crate is.
const crates = new Map(
  [...cargoLock.matchAll(/\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)"/g)].map((m) => [m[1], m[2]]),
);
const npmPackages = JSON.parse(packageLock).packages;
const npmVersion = (name) => npmPackages[`node_modules/${name}`]?.version;

// Each npm side Driveby installs, paired with the crate it must match.
const pairs = Object.keys(npmPackages)
  .map((path) => path.replace(/^node_modules\//, ''))
  .filter((name) => name === '@tauri-apps/api' || /^@tauri-apps\/plugin-[a-z-]+$/.test(name))
  .map((npm) => [npm === '@tauri-apps/api' ? 'tauri' : npm.replace('@tauri-apps/plugin-', 'tauri-plugin-'), npm]);

describe('Tauri versions', () => {
  test('the core and every plugin are in both lockfiles', () => {
    expect(pairs.map(([, npm]) => npm)).toContain('@tauri-apps/api');
    expect(pairs.length, 'the api package and at least one plugin').toBeGreaterThan(1);
    for (const [crate, npm] of pairs) {
      expect(crates.get(crate), `${crate} in src-tauri/Cargo.lock (npm has ${npm})`).toBeTruthy();
    }
  });

  test.each(pairs)('%s and %s are on the same major.minor', (crate, npm) => {
    const rust = crates.get(crate);
    const js = npmVersion(npm);
    expect(majorMinor(js), `${crate} ${rust} needs ${npm} ${majorMinor(rust)}.x`).toBe(majorMinor(rust));
  });
});
