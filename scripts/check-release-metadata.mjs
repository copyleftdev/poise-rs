#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const manifestPath = resolve(root, "Cargo.toml");
const manifest = readFileSync(manifestPath, "utf8");
const publishable = process.argv.includes("--publishable");
const errors = [];

const packageNames = [
  "poise-core",
  "poise-discovery",
  "poise-health",
  "poise-tower",
  "poise-tokio",
  "poise-observe",
];

const workspacePackage = manifest.match(/\[workspace\.package\]([\s\S]*?)(?=\n\[|$)/)?.[1] ?? "";
const version = workspacePackage.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  errors.push("[workspace.package].version must be an explicit SemVer version");
}

for (const name of packageNames) {
  const dependencyPattern = new RegExp(`^${name}\\s*=\\s*\\{[^}]*version\\s*=\\s*"([^"]+)"[^}]*\\}`, "m");
  const dependencyVersion = manifest.match(dependencyPattern)?.[1];
  if (dependencyVersion !== version) {
    errors.push(`${name} workspace dependency is ${dependencyVersion ?? "missing"}; expected ${version}`);
  }

  const member = readFileSync(resolve(root, "crates", name, "Cargo.toml"), "utf8");
  if (!/^version\.workspace\s*=\s*true/m.test(member)) {
    errors.push(`${name} must inherit the workspace version`);
  }
  if (!/^publish\.workspace\s*=\s*true/m.test(member)) {
    errors.push(`${name} must inherit the workspace publish switch`);
  }
}

if (publishable) {
  if (!/^publish\s*=\s*true/m.test(workspacePackage)) {
    errors.push("workspace publication is disabled; set [workspace.package].publish = true only for an approved release");
  }
  if (!/^(license|license-file)\s*=\s*"[^"]+"/m.test(workspacePackage)) {
    errors.push("workspace license or license-file metadata is required");
  }
  if (!/^repository\s*=\s*"https:\/\/github\.com\/[^/]+\/[^/]+\/?"/m.test(workspacePackage)) {
    errors.push("workspace repository must be the final GitHub HTTPS URL");
  }
  if (!existsSync(resolve(root, "LICENSE-APACHE")) && !existsSync(resolve(root, "LICENSE-MIT")) && !existsSync(resolve(root, "LICENSE"))) {
    errors.push("an explicit license file must exist before publication");
  }
  for (const name of packageNames) {
    const member = readFileSync(resolve(root, "crates", name, "Cargo.toml"), "utf8");
    if (!/^license(?:-file)?\.workspace\s*=\s*true/m.test(member)) {
      errors.push(`${name} must inherit workspace license metadata`);
    }
    if (!/^repository\.workspace\s*=\s*true/m.test(member)) {
      errors.push(`${name} must inherit the workspace repository URL`);
    }
  }
}

if (errors.length > 0) {
  console.error("release metadata check failed:");
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log(`release metadata: coherent workspace version ${version}${publishable ? " and publishable" : " (publishing remains gated)"}`);
