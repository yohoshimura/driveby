#!/usr/bin/env node
// Delete build output so the working copy goes back to source-only.
//
// A full Rust debug + release build of this project reaches ~9 GB, which is
// three orders of magnitude more than the sources. This removes it without
// touching anything git tracks.
//
//   npm run clean            build output only (target, dist, gen)
//   npm run clean:all        the above plus node_modules
//   npm run clean -- -n      show what would go, delete nothing
//
// release-artifacts/ is never touched: it holds signed installers that only
// exist locally, and re-creating them costs a full build plus the signing key.
//
// Not named "uninstall": npm reserves preuninstall/uninstall/postuninstall as
// lifecycle hooks and would run a script by that name on its own.

import { existsSync, readdirSync, readFileSync, rmSync, statSync, statfsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// Paths are hardcoded rather than taken from argv: this script deletes
// recursively and never needs to point anywhere the repo doesn't own.
const BUILD_OUTPUT = [
  ['src-tauri/target', 'Rust build cache', 'cargo rebuilds it'],
  ['dist', 'Vite bundle', 'npm run build'],
  ['src-tauri/gen', 'Tauri generated schemas', 'regenerated on the next build'],
];
const DEPENDENCIES = [
  ['node_modules', 'npm dependencies', 'npm install'],
];

const NEVER_DELETE = ['release-artifacts'];

const argv = process.argv.slice(2);
const has = (...names) => names.some((n) => argv.includes(n));

if (has('-h', '--help')) {
  console.log(`
  npm run clean              remove build output (target, dist, gen)
  npm run clean:all          also remove node_modules
  npm run clean -- -n        dry run: list targets and sizes, delete nothing

  release-artifacts/ is always preserved.
`);
  process.exit(0);
}

const dryRun = has('-n', '--dry-run');
const withDeps = has('-a', '--all');
const targets = withDeps ? [...BUILD_OUTPUT, ...DEPENDENCIES] : BUILD_OUTPUT;

// --- guards ----------------------------------------------------------------

function die(msg) {
  console.error(`clean: ${msg}`);
  process.exit(1);
}

// Refuse to run anywhere but this repo, so a stray copy of the script can't
// delete a node_modules that belongs to another project.
let pkg;
try {
  pkg = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'));
} catch {
  die(`no package.json at ${ROOT}`);
}
if (pkg.name !== 'driveby' || !existsSync(join(ROOT, 'src-tauri'))) {
  die(`${ROOT} does not look like the driveby repo`);
}

/** Tracked files under a path, or null when git can't answer. */
function trackedCount(rel) {
  try {
    const out = execFileSync('git', ['ls-files', '--', rel], { cwd: ROOT, encoding: 'utf8' });
    return out.split('\n').filter(Boolean).length;
  } catch {
    return null;
  }
}

function dirSize(abs) {
  let bytes = 0;
  const stack = [abs];
  while (stack.length) {
    const dir = stack.pop();
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of entries) {
      const p = join(dir, e.name);
      if (e.isDirectory() && !e.isSymbolicLink()) stack.push(p);
      else if (e.isFile()) {
        try {
          bytes += statSync(p).size;
        } catch {
          /* vanished mid-walk */
        }
      }
    }
  }
  return bytes;
}

function human(bytes) {
  if (bytes == null) return '?';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

function freeBytes() {
  try {
    const s = statfsSync(ROOT);
    return Number(s.bfree) * Number(s.bsize);
  } catch {
    return null;
  }
}

function removeDir(abs) {
  if (process.platform === 'win32') {
    // rmdir clears a multi-GB tree in seconds; fs.rmSync walks every entry
    // from Node and takes minutes on the same tree.
    try {
      execFileSync('cmd', ['/c', 'rmdir', '/s', '/q', abs], { stdio: 'ignore' });
      if (!existsSync(abs)) return;
    } catch {
      /* locked or partially removed — fall through for a real error */
    }
  }
  rmSync(abs, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 });
}

// --- run -------------------------------------------------------------------

const present = [];
let gitAnswered = true;

for (const [rel, label, restore] of targets) {
  const abs = resolve(ROOT, rel);

  if (!abs.startsWith(ROOT + sep)) die(`${rel} resolves outside the repo`);
  if (NEVER_DELETE.some((p) => abs === resolve(ROOT, p) || abs.startsWith(resolve(ROOT, p) + sep))) {
    die(`${rel} is on the protected list`);
  }
  if (!existsSync(abs)) continue;

  const tracked = trackedCount(rel);
  if (tracked === null) gitAnswered = false;
  else if (tracked > 0) die(`${rel} holds ${tracked} git-tracked file(s) — refusing to delete`);

  present.push({ rel, abs, label, restore });
}

if (!present.length) {
  console.log('Nothing to clean — no build output present.');
  process.exit(0);
}

if (!gitAnswered) {
  console.warn('clean: git unavailable, relying on the hardcoded path list alone.\n');
}

if (dryRun) {
  console.log('Dry run — nothing will be deleted.\n');
  let total = 0;
  for (const t of present) {
    const size = dirSize(t.abs);
    total += size;
    console.log(`  ${t.rel.padEnd(20)} ${human(size).padStart(9)}   ${t.label}`);
  }
  console.log(`\n  ${'total'.padEnd(20)} ${human(total).padStart(9)}`);
  console.log('\nRun without -n to delete.');
  process.exit(0);
}

const before = freeBytes();

for (const t of present) {
  process.stdout.write(`  removing ${t.rel} ... `);
  try {
    removeDir(t.abs);
    console.log('done');
  } catch (e) {
    console.log('FAILED');
    die(`could not remove ${t.rel}: ${e.message}\n       a file inside is probably locked by a running process`);
  }
}

const after = freeBytes();
const freed = before != null && after != null ? Math.max(0, after - before) : null;

console.log(`\nRemoved ${present.length} path(s)${freed != null ? `, ${human(freed)} reclaimed` : ''}.`);
console.log(`Restore with: ${[...new Set(present.map((t) => t.restore))].join(', ')}`);
if (!withDeps && existsSync(join(ROOT, 'node_modules'))) {
  console.log('node_modules kept — use npm run clean:all to remove it too.');
}
