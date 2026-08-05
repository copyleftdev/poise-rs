#!/usr/bin/env node

import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const output = process.env.POISE_SHOWCASE_OUTPUT
  ? resolve(process.env.POISE_SHOWCASE_OUTPUT)
  : join(root, "site", "data", "latest.json");

function rustFiles(directory, results = []) {
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    const stats = statSync(path);
    if (stats.isDirectory()) rustFiles(path, results);
    else if (entry.endsWith(".rs")) results.push(path);
  }
  return results;
}

function countMatches(paths, pattern) {
  let count = 0;
  for (const path of paths) count += readFileSync(path, "utf8").match(pattern)?.length ?? 0;
  return count;
}

function stepStatus(name) {
  const status = process.env[name];
  if (status === "success") return "passed";
  if (status === "failure" || status === "cancelled") return "failed";
  return "unavailable";
}

function lineCount(filename) {
  try {
    return readFileSync(join(root, "mutants.out", filename), "utf8")
      .split("\n")
      .filter((line) => line.trim().length > 0).length;
  } catch {
    return 0;
  }
}

const files = rustFiles(join(root, "crates"));
const propertyFiles = files.filter((path) => /tests\/property_[^/]+\.rs$/.test(path));
const loomFiles = files.filter((path) => /tests\/loom_[^/]+\.rs$/.test(path));
const testEntries = countMatches(files, /#\s*\[test\]/g);
const propertyLaws = countMatches(propertyFiles, /#\s*\[test\]/g);
const loomModels = countMatches(loomFiles, /loom::model/g);
const casesPerLaw = Number.parseInt(process.env.POISE_PROPTEST_CASES || "256", 10);

const mutation = {
  caught: lineCount("caught.txt"),
  missed: lineCount("missed.txt"),
  timeout: lineCount("timeout.txt"),
  unviable: lineCount("unviable.txt"),
};
mutation.sites = mutation.caught + mutation.missed + mutation.timeout + mutation.unviable;

const statuses = {
  ordinary: stepStatus("POISE_ORDINARY_OUTCOME"),
  property: stepStatus("POISE_PROPERTY_OUTCOME"),
  mutation: stepStatus("POISE_MUTATION_OUTCOME"),
  loom: stepStatus("POISE_LOOM_OUTCOME"),
  msrv: stepStatus("POISE_MSRV_OUTCOME"),
};
const statusValues = Object.values(statuses);
const overallStatus = statusValues.includes("failed")
  ? "failed"
  : statusValues.every((status) => status === "passed") ? "passed" : "incomplete";

const repository = process.env.GITHUB_REPOSITORY
  ? `${process.env.GITHUB_SERVER_URL || "https://github.com"}/${process.env.GITHUB_REPOSITORY}`
  : "";
const revision = process.env.GITHUB_SHA?.slice(0, 12) || "local source snapshot";
const sourceUrl = repository && process.env.GITHUB_RUN_ID
  ? `${repository}/actions/runs/${process.env.GITHUB_RUN_ID}`
  : "";

const record = {
  schemaVersion: 1,
  provenance: {
    kind: process.env.GITHUB_ACTIONS === "true" ? "ci" : "recorded",
    label: process.env.GITHUB_ACTIONS === "true" ? "Live GitHub Actions verification" : "Recorded local verification",
    generatedAt: new Date().toISOString(),
    revision,
    sourceUrl,
  },
  project: {
    repository,
    docs: repository ? `${repository}/blob/main/docs/architecture.md` : "../docs/architecture.md",
  },
  verification: {
    status: overallStatus,
    ordinary: { value: testEntries, detail: "authored test entry points", status: statuses.ordinary },
    property: { value: propertyLaws * casesPerLaw, laws: propertyLaws, casesPerLaw, status: statuses.property },
    mutation: { ...mutation, status: statuses.mutation },
    loom: { value: loomModels, status: statuses.loom },
    msrv: { value: "1.85", status: statuses.msrv },
  },
  diagnostics: {
    generator: relative(root, import.meta.filename),
    note: "Counts describe authored proof surfaces; statuses come from the associated CI steps.",
  },
};

writeFileSync(output, `${JSON.stringify(record, null, 2)}\n`, "utf8");
console.log(`wrote ${relative(root, output)} (${overallStatus})`);
