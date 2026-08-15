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

import { execFile } from "node:child_process";
import { access, readdir, readFile } from "node:fs/promises";
import { promisify } from "node:util";
import { extname, join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");

const run = promisify(execFile);

/**
 * The subset of `paths` that git ignores.
 *
 * Generated output is named legitimately by documentation -- the book builds to
 * `site/book/`, which the prose says and a clean checkout does not contain --
 * and its presence depends on whether anyone has run a build. Checking
 * existence for those makes the gate report a different answer on a developer
 * machine than in CI, which is the one thing a gate must never do. Asking git
 * is environment-independent: ignored means generated, and generated means
 * unverifiable here rather than missing.
 */
export async function ignoredPaths(paths) {
  if (paths.length === 0) {
    return new Set();
  }
  try {
    // Paths as arguments rather than on stdin: `execFile` has no way to write
    // to a child's stdin, so `--stdin` waits for input that never arrives.
    const { stdout } = await run("git", ["check-ignore", "--", ...paths]);
    return new Set(stdout.split("\n").filter(Boolean));
  } catch (error) {
    // `check-ignore` exits non-zero when nothing matched, which is a result
    // rather than a failure. Anything else means git could not answer, and the
    // check falls back to existence alone rather than passing everything.
    if (error.code === 1) {
      return new Set((error.stdout ?? "").split("\n").filter(Boolean));
    }
    return new Set();
  }
}

/** Whether a path exists, without distinguishing files from directories. */
async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

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


/**
 * Markdown links to files in this repository, with any fragment removed.
 *
 * External URLs and bare anchors are somebody else's problem: the first cannot
 * be checked offline, and the second is already covered for chapters by the
 * book validator.
 */
export function localLinks(document) {
  // The angle-bracket form is valid Markdown and permits spaces in the path,
  // so it cannot share the whitespace-terminated pattern with the bare form.
  return [...document.matchAll(/!?\[[^\]]*\]\(\s*(?:<([^>]*)>|([^)\s]+))/g)]
    .map((match) => (match[1] ?? match[2]).trim())
    .filter((target) => target && !/^(?:[a-z][a-z+.-]*:|\/\/|#)/i.test(target))
    .map((target) => (target.includes("#") ? target.slice(0, target.indexOf("#")) : target))
    .filter(Boolean);
}

/**
 * Repository paths named anywhere in a document, fenced code included.
 *
 * The book validator strips fenced blocks before checking links, which is right
 * for links and wrong for this: a chapter telling a reader to run a script is
 * making a claim about the tree, and it is the claim most likely to rot after a
 * rename. Globs are skipped because they name a set rather than a file.
 */
export function referencedPaths(document) {
  const roots = "scripts|crates|docs|fuzz|book-theme|site";
  const pattern = new RegExp(String.raw`(?<![\w/.-])((?:${roots})/[\w./-]*[\w/])`, "g");
  return [...document.matchAll(pattern)]
    .map((match) => match[1])
    .filter((path) => !path.includes("*") && !path.includes("?"));
}


/**
 * Job names a workflow defines, with any matrix expanded.
 *
 * Job names are the contexts branch protection requires, so they are one fact
 * with three copies in this repository: the workflow that defines them, the
 * protection script that requires them, and the release chapter that lists
 * them. Renaming a job silently desynchronises the other two, and the
 * consequence is not cosmetic -- a required context that no longer reports
 * blocks every merge, and a job that quietly stops being required stops
 * gating.
 *
 * Deliberately narrow parsing rather than a YAML dependency: a job name is a
 * `name:` at job indentation, which a step's `- name:` is not.
 */
export function workflowJobNames(workflow) {
  const names = [...workflow.matchAll(/^ {4}name: (.+)$/gm)].map((match) =>
    match[1].trim().replace(/^["']|["']$/g, ""),
  );

  return names.flatMap((name) => {
    const matrix = name.match(/\$\{\{\s*matrix\.(\w+)\s*\}\}/);
    if (!matrix) {
      return [name];
    }
    const values = workflow.match(
      new RegExp(String.raw`^\s*${matrix[1]}: \[([^\]]*)\]`, "m"),
    );
    if (!values) {
      return [name];
    }
    return values[1]
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean)
      .map((value) => name.replace(matrix[0], value));
  });
}

/**
 * The workspace MSRV, and every place the prose or CI restates it.
 *
 * One fact with five copies: the manifest declares it, a badge shows it, the
 * README says it, the test matrix pins it, and branch protection names the
 * resulting job. The manifest is the only one a compiler enforces, so it is the
 * source and the rest are checked against it.
 */
export function msrvClaims({ manifest, readme, workflow }) {
  const declared = manifest.match(/^rust-version = "([\d.]+)"$/m)?.[1];
  if (!declared) {
    throw new Error("Cargo.toml declares no rust-version for the drift check to compare");
  }
  const claims = [
    { label: "README badge", value: readme.match(/MSRV-([\d.]+)/)?.[1] },
    {
      label: "README text",
      value: readme.match(/Minimum supported Rust version: \*\*([\d.]+)\*\*/)?.[1],
    },
    // The matrix pins a patch release, so it must start with the declared
    // version rather than equal it.
    { label: "CI matrix", value: workflow.match(/toolchain: \[stable, ([\d.]+)\]/)?.[1], prefix: true },
  ];
  return { declared, claims };
}

/** Status-check contexts the protection script requires. */
export function protectedContexts(script) {
  const json = script.match(/<<'JSON'\n([\s\S]*?)\nJSON/);
  if (!json) {
    throw new Error(
      "scripts/protect-main.sh no longer embeds a JSON payload the drift check " +
        "can read; update the pattern alongside the script",
    );
  }
  return JSON.parse(json[1]).required_status_checks.contexts;
}

/** Checks the release chapter lists exactly the required contexts. */
export function documentedContexts(chapter) {
  const section = chapter.match(
    /Protect `main` with required pull requests and these CI jobs:\n\n((?:- .+\n)+)/,
  );
  if (!section) {
    throw new Error(
      "docs/releasing.md no longer lists the required CI jobs where the drift " +
        "check looks; update the pattern alongside the prose",
    );
  }
  return section[1]
    .split("\n")
    .filter(Boolean)
    .map((line) => line.replace(/^- /, "").trim());
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

  // Documents outside docs/ that the book validator never sees, plus the
  // chapters themselves for the fenced references it strips.
  const prose = [
    "README.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "CHANGELOG.md",
    ...(await walk(join(root, "docs"), [".md"])).map((path) =>
      path.slice(root.length + 1),
    ),
  ];

  const named = new Map();
  for (const file of prose) {
    const document = await readFile(join(root, file), "utf8").catch(() => null);
    if (document !== null) {
      for (const target of referencedPaths(document)) {
        if (!named.has(target)) {
          named.set(target, file);
        }
      }
    }
  }
  const generated = await ignoredPaths([...named.keys()]);

  let links = 0;
  let references = 0;
  for (const file of prose) {
    const document = await readFile(join(root, file), "utf8").catch(() => null);
    if (document === null) {
      problems.push(`${file}: listed for checking but absent`);
      continue;
    }
    for (const target of localLinks(document)) {
      links += 1;
      const resolved = resolve(join(root, file), "..", target);
      if (!(await exists(resolved))) {
        problems.push(`${file}: links to ${target}, which does not exist`);
      }
    }
    for (const target of referencedPaths(document)) {
      if (generated.has(target)) {
        continue;
      }
      references += 1;
      if (!(await exists(join(root, target)))) {
        problems.push(`${file}: names ${target}, which does not exist`);
      }
    }
  }

  // One fact, five copies: manifest, badge, prose, test matrix, required job.
  try {
    const { declared, claims } = msrvClaims({
      manifest: await readFile(join(root, "Cargo.toml"), "utf8"),
      readme: await readFile(join(root, "README.md"), "utf8"),
      workflow: await readFile(join(root, ".github/workflows/ci.yml"), "utf8"),
    });
    for (const claim of claims) {
      if (claim.value === undefined) {
        problems.push(`${claim.label}: no MSRV found where the drift check looks`);
        continue;
      }
      const agrees = claim.prefix
        ? claim.value.startsWith(declared)
        : claim.value === declared;
      if (!agrees) {
        problems.push(
          `${claim.label} says MSRV ${claim.value}, Cargo.toml declares ${declared}`,
        );
      }
    }
  } catch (error) {
    problems.push(error.message);
  }

  // One fact, three copies: the workflows define the job names, the protection
  // script requires them, and the release chapter lists them.
  const workflows = await Promise.all(
    ["ci.yml", "security.yml"].map((file) =>
      readFile(join(root, ".github/workflows", file), "utf8"),
    ),
  );
  const defined = new Set(workflows.flatMap(workflowJobNames));
  let contexts = 0;
  try {
    const required = protectedContexts(
      await readFile(join(root, "scripts/protect-main.sh"), "utf8"),
    );
    const listed = documentedContexts(await readFile(join(root, "docs/releasing.md"), "utf8"));
    contexts = required.length;

    for (const context of required) {
      if (!defined.has(context)) {
        problems.push(
          `scripts/protect-main.sh requires "${context}", which no workflow defines`,
        );
      }
    }
    const requiredSet = new Set(required);
    for (const context of listed) {
      if (!requiredSet.has(context)) {
        problems.push(
          `docs/releasing.md lists "${context}", which protect-main.sh does not require`,
        );
      }
    }
    for (const context of required) {
      if (!listed.includes(context)) {
        problems.push(
          `docs/releasing.md omits "${context}", which protect-main.sh requires`,
        );
      }
    }
  } catch (error) {
    problems.push(error.message);
  }

  if (problems.length > 0) {
    console.error("documentation drifted from the tree it describes:");
    for (const problem of problems) {
      console.error(`  ${problem}`);
    }
    process.exit(1);
  }

  console.log(
    `documentation matches the tree: ${claims.length} counted claims, ` +
      `${links} local links, ${references} named paths ` +
      `(${generated.size} generated, skipped), ${contexts} required checks ` +
      "agreeing across workflow, protection script, and chapter, a consistent " +
      "MSRV, and the recorded " +
      "campaign breakdown",
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  await main();
}
