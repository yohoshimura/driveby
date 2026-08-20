#!/usr/bin/env node
// The version lives in two manifests: package.json (which vite injects as
// __APP_VERSION__ for the UI) and Cargo.toml (which Tauri reads for the
// bundle, and main.rs logs via CARGO_PKG_VERSION). tauri.conf.json
// deliberately has no "version" key so it can't drift from Cargo.toml.
//
//   node scripts/bump-version.mjs 1.6.1

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const version = process.argv[2];

if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? '')) {
  console.error('usage: node scripts/bump-version.mjs <x.y.z>');
  process.exit(1);
}

const pkgPath = join(root, 'package.json');
const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
pkg.version = version;
writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);

const cargoPath = join(root, 'src-tauri', 'Cargo.toml');
const cargo = readFileSync(cargoPath, 'utf8');
// Only the [package] version — the first `version = ` line — not any
// dependency's.
const bumped = cargo.replace(/^version = ".*"$/m, `version = "${version}"`);
if (bumped === cargo) {
  console.error('could not find the [package] version line in src-tauri/Cargo.toml');
  process.exit(1);
}
writeFileSync(cargoPath, bumped);

console.log(`version set to ${version} in package.json and src-tauri/Cargo.toml`);
console.log('next: commit, then tag with  git tag v' + version);
