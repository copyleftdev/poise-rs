#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const manifestPath = resolve(root, "Cargo.toml");
const requested = process.argv.find((argument) => !argument.startsWith("--") && argument !== process.argv[0] && argument !== process.argv[1]);
const allowDirty = process.argv.includes("--allow-dirty");
const dryRun = process.argv.includes("--dry-run");

if (!requested) {
  console.error("usage: node scripts/bump-version.mjs <patch|minor|major|X.Y.Z> [--allow-dirty]");
  process.exit(2);
}

const gitProbe = spawnSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" });
if (gitProbe.status === 0 && gitProbe.stdout.trim() && !allowDirty) {
  console.error("version bump refused: working tree is dirty (pass --allow-dirty only after reviewing the diff)");
  process.exit(1);
}

let manifest = readFileSync(manifestPath, "utf8");
const workspacePackagePattern = /(\[workspace\.package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m;
const current = manifest.match(workspacePackagePattern)?.[2];
if (!current) {
  console.error("version bump refused: workspace package version was not found");
  process.exit(1);
}

const match = current.match(/^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$/);
if (!match) {
  console.error(`version bump refused: ${current} is not supported SemVer`);
  process.exit(1);
}

let next = requested;
if (["patch", "minor", "major"].includes(requested)) {
  let major = Number(match[1]);
  let minor = Number(match[2]);
  let patch = Number(match[3]);
  if (requested === "patch") patch += 1;
  if (requested === "minor") { minor += 1; patch = 0; }
  if (requested === "major") { major += 1; minor = 0; patch = 0; }
  next = `${major}.${minor}.${patch}`;
}

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(next)) {
  console.error(`version bump refused: ${next} is not supported SemVer`);
  process.exit(2);
}
if (next === current) {
  console.error(`version bump refused: ${next} is already the workspace version`);
  process.exit(1);
}

manifest = manifest.replace(workspacePackagePattern, `$1${next}$3`);
for (const name of ["poise-core", "poise-discovery", "poise-health", "poise-observe", "poise-tokio", "poise-tower"]) {
  const dependencyPattern = new RegExp(`(^${name}\\s*=\\s*\\{[^}]*version\\s*=\\s*")${current}("[^}]*\\})`, "m");
  if (!dependencyPattern.test(manifest)) {
    console.error(`version bump refused: ${name} does not reference workspace version ${current}`);
    process.exit(1);
  }
  manifest = manifest.replace(dependencyPattern, `$1${next}$2`);
}

if (dryRun) {
  console.log(`Poise workspace version dry run: ${current} -> ${next}`);
  process.exit(0);
}

writeFileSync(manifestPath, manifest, "utf8");

const check = spawnSync("cargo", ["check", "--workspace", "--all-features"], {
  cwd: root,
  stdio: "inherit",
});
if (check.status !== 0) {
  console.error("version files were updated, but cargo check failed; review before committing");
  process.exit(check.status ?? 1);
}

const metadata = spawnSync(process.execPath, [resolve(root, "scripts", "check-release-metadata.mjs")], {
  cwd: root,
  stdio: "inherit",
});
if (metadata.status !== 0) process.exit(metadata.status ?? 1);

console.log(`Poise workspace version: ${current} -> ${next}`);
console.log("Review Cargo.toml, Cargo.lock, and CHANGELOG.md before committing.");
