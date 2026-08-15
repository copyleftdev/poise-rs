#!/usr/bin/env node

// Verifies the counted claims in the prose against the tree they describe.
//
// Every number in a README is a claim with an expiry date nobody wrote down.
// This repository has already shipped several past theirs: ten Loom models when
// there were thirteen, 240 test entry points when there were 259, 642 mutation
// sites when there were 705, and a performance chapter calling criterion
// baselines a roadmap item after they had landed. Each was true when written.
// That is the whole problem -- prose drifts silently, and the drift is invisible
// precisely because the sentence still reads correctly.
//
// Two kinds of claim, treated differently.
//
// Derivable claims are counted from the source and must match exactly. A
// pattern that fails to match, or matches more than once, is also an error:
// rewording a sentence must break the check rather than quietly disable it,
// which is the failure mode a checker is most likely to acquire.
//
// Recorded claims cannot be derived here, because they come from a campaign
// this script does not run. Those are checked for internal consistency instead
// -- a breakdown that no longer sums to its own total is drift that arithmetic
// can catch without rerunning anything.

import { readdir, readFile } from "node:fs/promises";
import { extname, join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");

/** Every file under `directory` with one of `extensions`. */
async function walk(directory, extensions) {
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "target" || entry.name === "node_modules") {
        continue;
      }
      found.push(...(await walk(path, extensions)));
    } else if (extensions.includes(extname(entry.name))) {
      found.push(path);
    }
  }
  return found;
}

/**
 * Counts test entry points across the workspace sources.
 *
 * Both the plain attribute and the runtime-provided ones, because the claim is
 * about authored entry points rather than about a spelling. Counting `#[test]`
 * alone undercounts by every async test, which is a defect the first version of
 * this had: the check would have held a wrong number steady rather than
 * catching it.
 */
export function countTestAttributes(sources) {
  return sources.reduce(
    (total, source) =>
      total + (source.match(/#\[(?:test\]|[\w:]*::test(?:\([^)]*\))?\])/g)?.length ?? 0),
    0,
  );
}

/**
 * Counts laws declared inside `proptest!` blocks.
 *
 * A law is a function inside the macro, not the macro itself, so counting
 * `proptest!` would undercount by however many laws share a block.
 */
export function countProptestLaws(sources) {
  let laws = 0;
  for (const source of sources) {
    let inside = false;
    let depth = 0;
    for (const line of source.split("\n")) {
      if (!inside && /proptest!\s*\{/.test(line)) {
        inside = true;
        depth = 1;
        continue;
      }
      if (!inside) {
        continue;
      }
      depth += (line.match(/\{/g)?.length ?? 0) - (line.match(/\}/g)?.length ?? 0);
      if (depth <= 0) {
        inside = false;
        continue;
      }
      if (/^\s*fn\s+\w+/.test(line)) {
        laws += 1;
      }
    }
  }
  return laws;
}

/** Counts `loom::model` invocations, which is one per model. */
export function countLoomModels(sources) {
  return sources.reduce(
    (total, source) => total + (source.match(/loom::model/g)?.length ?? 0),
    0,
  );
}

const NUMBER_WORDS = [
  "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
  "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen",
  "sixteen", "seventeen", "eighteen", "nineteen", "twenty",
];

/** Renders a count the way the prose spells it. */
export function spell(count, style) {
  if (style !== "word") {
    return String(count);
  }
  const word = NUMBER_WORDS[count];
  if (word === undefined) {
    throw new RangeError(`no spelled form for ${count}; write the claim in digits`);
  }
  return word;
}

/**
 * Finds a claim's stated value, insisting the pattern match exactly once.
 *
 * Zero matches means the sentence was reworded and this check silently stopped
 * checking. More than one means the claim is stated twice and only one of them
 * would have been corrected.
 */
export function statedValue(document, { file, pattern, label }) {
  const matches = [...document.matchAll(pattern)];
  if (matches.length === 0) {
    throw new Error(
      `${file}: no sentence matched the ${label} claim; the pattern in ` +
        "check-docs-drift.mjs needs updating alongside the prose",
    );
  }
  if (matches.length > 1) {
    throw new Error(
      `${file}: the ${label} claim appears ${matches.length} times; state it once ` +
        "so correcting it cannot leave a stale copy",
    );
  }
  return matches[0][1];
}

/** Checks that a stated breakdown still sums to its stated total. */
export function checkSum(label, total, parts) {
  const sum = parts.reduce((running, part) => running + part.value, 0);
  if (sum !== total) {
    const detail = parts.map((part) => `${part.label} ${part.value}`).join(" + ");
    return `${label}: ${detail} = ${sum}, but the total is stated as ${total}`;
  }
  return null;
}

async function main() {
  const rustSources = await Promise.all(
    (await walk(join(root, "crates"), [".rs"])).map((path) => readFile(path, "utf8")),
  );
  const loomSources = await Promise.all(
    (await walk(join(root, "crates"), [".rs"]))
      .filter((path) => /loom_[^/]+\.rs$/.test(path))
      .map((path) => readFile(path, "utf8")),
  );
  const propertySources = await Promise.all(
    (await walk(join(root, "crates"), [".rs"]))
      .filter((path) => /property_[^/]+\.rs$/.test(path))
      .map((path) => readFile(path, "utf8")),
  );

  const claims = [
    {
      file: "README.md",
      label: "authored test entry points",
      pattern: /\*\*(\d+) authored test entry points\*\*/g,
      derived: countTestAttributes(rustSources),
      style: "digits",
    },
    {
      file: "README.md",
      label: "Proptest laws",
      pattern: /\*\*(\d+) Proptest laws\*\*/g,
      derived: countProptestLaws(propertySources),
      style: "digits",
    },
    {
      file: "README.md",
      label: "Loom models",
      pattern: /\*\*(\w+) Loom models\*\*/g,
      derived: countLoomModels(loomSources),
      style: "word",
    },
    {
      file: "docs/fuzzing.md",
      label: "Loom models",
      pattern: /Today's (\w+) models all complete/g,
      derived: countLoomModels(loomSources),
      style: "word",
    },
  ];

  const problems = [];
  const documents = new Map();

  for (const claim of claims) {
    if (!documents.has(claim.file)) {
      documents.set(claim.file, await readFile(join(root, claim.file), "utf8"));
    }
    let stated;
    try {
      stated = statedValue(documents.get(claim.file), claim);
    } catch (error) {
      problems.push(error.message);
      continue;
    }
    const expected = spell(claim.derived, claim.style);
    if (stated.toLowerCase() !== expected.toLowerCase()) {
      problems.push(
        `${claim.file}: claims ${stated} ${claim.label}, tree has ${expected}`,
      );
    }
  }

  // Recorded campaign evidence. Not derivable here, because this script does not
  // run a mutation campaign; its breakdown must at least still add up.
  const mutation = await readFile(join(root, "docs/mutation-testing.md"), "utf8");
  const recorded =
    /examine (\d+) sites\. A\s+complete campaign caught (\d+), missed zero, found (\d+) unviable, and detected (\d+)/;
  const found = mutation.match(recorded);
  if (!found) {
    problems.push(
      "docs/mutation-testing.md: the recorded campaign breakdown no longer matches " +
        "its pattern, so its arithmetic is unchecked",
    );
  } else {
    const [, sites, caught, unviable, timeouts] = found.map(Number);
    const mismatch = checkSum("recorded campaign", sites, [
      { label: "caught", value: caught },
      { label: "missed", value: 0 },
      { label: "unviable", value: unviable },
      { label: "timeouts", value: timeouts },
    ]);
    if (mismatch) {
      problems.push(`docs/mutation-testing.md: ${mismatch}`);
    }
  }

  if (problems.length > 0) {
    console.error("documentation drifted from the tree it describes:");
    for (const problem of problems) {
      console.error(`  ${problem}`);
    }
    process.exit(1);
  }

  console.log(
    `documentation matches the tree: ${claims.length} counted claims and the ` +
      "recorded campaign breakdown",
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  await main();
}
