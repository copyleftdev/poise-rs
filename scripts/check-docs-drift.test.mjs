import assert from "node:assert/strict";
import test from "node:test";

import {
  checkSum,
  documentedContexts,
  ignoredPaths,
  localLinks,
  msrvClaims,
  nodeVersionPins,
  protectedContexts,
  reachesNode,
  referencedPaths,
  countLoomModels,
  countProptestLaws,
  countTestAttributes,
  spell,
  statedValue,
  workflowJobNames,
} from "./check-docs-drift.mjs";

test("test attributes are counted across sources", () => {
  const sources = ["#[test]\nfn a() {}\n#[test]\nfn b() {}", "#[test]\nfn c() {}"];
  assert.equal(countTestAttributes(sources), 3);
});

test("proptest laws are the functions inside the macro, not the macro", () => {
  // Counting `proptest!` would report one where there are three.
  const source = `
fn helper() {}

proptest! {
    #![proptest_config(Config::default())]

    fn first(x in 0..10) { }

    fn second(y in 0..10) { }

    fn third(z in 0..10) { }
}
`;
  assert.equal(countProptestLaws([source]), 3);
});

test("helpers outside the macro are not counted as laws", () => {
  // This is the mistake that made a hand count report seventeen laws where
  // there were fourteen: once concatenated, later files' helpers looked as
  // though they sat inside an earlier file's block.
  const source = `
proptest! {
    fn inside(x in 0..10) { }
}

fn after_the_block() {}
fn also_after() {}
`;
  assert.equal(countProptestLaws([source]), 1);
});

test("laws in separate blocks all count", () => {
  const source = `
proptest! {
    fn one(x in 0..10) { }
}

fn helper() {}

proptest! {
    fn two(y in 0..10) { }
}
`;
  assert.equal(countProptestLaws([source]), 2);
});

test("loom models are counted per invocation", () => {
  const source = "loom::model(|| {});\nloom::model(|| {});";
  assert.equal(countLoomModels([source]), 2);
});

test("counts are spelled the way the prose spells them", () => {
  assert.equal(spell(13, "word"), "thirteen");
  assert.equal(spell(13, "digits"), "13");
  assert.throws(() => spell(21, "word"), RangeError);
});

test("a claim that no longer matches its pattern is an error", () => {
  // Silent failure to check is the failure mode a checker acquires by drifting
  // out of step with the prose it checks, so it has to be louder than a pass.
  assert.throws(
    () =>
      statedValue("the prose was reworded", {
        file: "README.md",
        pattern: /\*\*(\d+) authored test entry points\*\*/g,
        label: "authored test entry points",
      }),
    /no sentence matched/,
  );
});

test("a claim stated twice is an error", () => {
  // Correcting one copy and leaving the other is worse than never checking,
  // because the corrected copy makes the document look maintained.
  const document = "**1 thing** and later **2 thing**";
  assert.throws(
    () =>
      statedValue(document, {
        file: "README.md",
        pattern: /\*\*(\d+) thing\*\*/g,
        label: "thing",
      }),
    /appears 2 times/,
  );
});

test("a matching claim yields its stated value", () => {
  const stated = statedValue("**283 authored test entry points** cover", {
    file: "README.md",
    pattern: /\*\*(\d+) authored test entry points\*\*/g,
    label: "authored test entry points",
  });
  assert.equal(stated, "283");
});

test("a breakdown that no longer sums to its total is reported", () => {
  const parts = [
    { label: "caught", value: 528 },
    { label: "missed", value: 0 },
    { label: "unviable", value: 154 },
    { label: "timeouts", value: 23 },
  ];
  assert.equal(checkSum("recorded campaign", 705, parts), null);
  assert.match(checkSum("recorded campaign", 700, parts), /= 705, but the total/);
});

test("async test attributes are entry points too", () => {
  // Counting only `#[test]` undercounts by every runtime-provided test, which
  // would have held a wrong number steady rather than catching it.
  const source = `
#[test]
fn plain() {}

#[tokio::test]
async fn async_plain() {}

#[tokio::test(flavor = "multi_thread")]
async fn async_configured() {}
`;
  assert.equal(countTestAttributes([source]), 3);
});

test("attributes that merely mention test are not counted", () => {
  const source = "#[cfg(test)]\nmod tests {}\n#[should_panic]\nfn nope() {}";
  assert.equal(countTestAttributes([source]), 0);
});

test("local links are extracted and external ones ignored", () => {
  const document = [
    "[chapter](docs/architecture.md)",
    "[anchored](docs/testing.md#gates)",
    "[external](https://example.com/docs/thing.md)",
    "[bare anchor](#section)",
    "[angle](<docs/spaced name.md>)",
  ].join("\n");

  assert.deepEqual(localLinks(document), [
    "docs/architecture.md",
    "docs/testing.md",
    "docs/spaced name.md",
  ]);
});

test("paths inside fenced code are found, since that is where commands live", () => {
  // The book validator strips fences before checking links, so a chapter
  // telling a reader to run a renamed script is exactly the claim it misses.
  const document = "```console\nscripts/bench.sh --baseline before\n```";
  assert.deepEqual(referencedPaths(document), ["scripts/bench.sh"]);
});

test("paths inside URLs are not treated as repository paths", () => {
  const document = "see https://github.com/owner/repo/docs/architecture.md for more";
  assert.deepEqual(referencedPaths(document), []);
});

test("globs name a set rather than a file and are skipped", () => {
  const document = "run `node --test scripts/*.test.mjs` and read docs/*.md";
  assert.deepEqual(referencedPaths(document), []);
});

test("trailing punctuation is not taken as part of the path", () => {
  const document = "Configured by scripts/protect-main.sh, then reviewed.";
  assert.deepEqual(referencedPaths(document), ["scripts/protect-main.sh"]);
});

test("generated paths are reported as skipped rather than silently dropped", async () => {
  // A gate that skips quietly is indistinguishable from one that stopped
  // checking, so the count of skipped paths belongs in the output.
  const ignored = await ignoredPaths(["site/book/", "scripts/check-docs-drift.mjs"]);

  assert.ok(ignored.has("site/book/"), "the built book is generated output");
  assert.ok(
    !ignored.has("scripts/check-docs-drift.mjs"),
    "a tracked script must still be checked",
  );
});

test("asking about no paths does not shell out", async () => {
  assert.equal((await ignoredPaths([])).size, 0);
});

test("job names are read at job indentation, not step indentation", () => {
  const workflow = [
    "jobs:",
    "  quality:",
    "    name: Format, lint, and docs",
    "    steps:",
    "      - name: A step that is not a job",
    "  book:",
    "    name: Documentation book",
  ].join("\n");

  assert.deepEqual(workflowJobNames(workflow), [
    "Format, lint, and docs",
    "Documentation book",
  ]);
});

test("a matrix job expands into the contexts it actually produces", () => {
  // Branch protection names the expanded contexts, so an unexpanded name would
  // never match anything and the check would be comparing a template.
  const workflow = [
    "jobs:",
    "  test:",
    "    name: Test / Rust ${{ matrix.toolchain }}",
    "    strategy:",
    "      matrix:",
    "        toolchain: [stable, 1.85.0]",
  ].join("\n");

  assert.deepEqual(workflowJobNames(workflow), [
    "Test / Rust stable",
    "Test / Rust 1.85.0",
  ]);
});

test("protected contexts are read from the script's own payload", () => {
  const script = [
    "gh api --method PUT foo --input - <<'JSON'",
    '{"required_status_checks":{"strict":true,"contexts":["One","Two"]}}',
    "JSON",
  ].join("\n");

  assert.deepEqual(protectedContexts(script), ["One", "Two"]);
});

test("a protection script the check can no longer read is an error", () => {
  // Silently finding nothing to compare would leave protection unchecked while
  // the gate still reported success.
  assert.throws(() => protectedContexts("gh api --method PUT foo"), /no longer embeds/);
});

test("a chapter that stops listing the jobs is an error", () => {
  assert.throws(() => documentedContexts("# Releasing\n\nNo list here.\n"), /no longer lists/);
});

test("the documented job list is read as written", () => {
  const chapter = [
    "Protect `main` with required pull requests and these CI jobs:",
    "",
    "- One",
    "- Two",
    "",
    "Then do something else.",
  ].join("\n");

  assert.deepEqual(documentedContexts(chapter), ["One", "Two"]);
});

test("an MSRV bump left half-applied is caught", () => {
  const { declared, claims } = msrvClaims({
    manifest: 'rust-version = "1.85"\n',
    readme: "MSRV-1.90 badge\nMinimum supported Rust version: **1.85**\n",
    workflow: "        toolchain: [stable, 1.85.0]\n",
  });

  assert.equal(declared, "1.85");
  const badge = claims.find((claim) => claim.label === "README badge");
  assert.equal(badge.value, "1.90");
  const matrix = claims.find((claim) => claim.label === "CI matrix");
  assert.ok(matrix.value.startsWith(declared), "the matrix pins a patch release");
});

test("an MSRV prefix must end on a component boundary", () => {
  // Without the boundary, a declared 1.85 accepts 1.850.0 and a declared
  // 1.85.1 accepts 1.85.10 -- distinct versions that share a text prefix.
  const boundary = (declared, value) =>
    value === declared || value.startsWith(`${declared}.`);

  assert.ok(boundary("1.85", "1.85"));
  assert.ok(boundary("1.85", "1.85.0"));
  assert.ok(!boundary("1.85", "1.850.0"));
  assert.ok(!boundary("1.85.1", "1.85.10"));
  assert.ok(!boundary("1.85", "1.9"));
});

test("a Node version written inline is distinguished from one read from a file", () => {
  const workflow = `
      - uses: actions/setup-node@abc # v4
        with:
          node-version: "22"
      - uses: actions/setup-node@abc # v4
        with:
          node-version-file: .nvmrc
`;

  assert.deepEqual(nodeVersionPins(workflow), [
    { inline: "22", file: undefined },
    { inline: undefined, file: ".nvmrc" },
  ]);
});

test("a workflow with no setup-node reports no pin at all", () => {
  // Distinct from pinning badly: nothing to compare, and the caller decides
  // whether that silence matters by asking whether the workflow reaches Node.
  assert.deepEqual(nodeVersionPins("      - run: cargo test\n"), []);
});

test("Node reached through a repository script is still Node", () => {
  // The dependency that hides: the workflow says only `run: scripts/build.sh`,
  // and the script inside it is what needs a version nobody pinned.
  const workflow = "      - run: scripts/build.sh\n";
  const scripts = { "scripts/build.sh": "#!/usr/bin/env bash\nnode --test scripts/x.test.mjs\n" };

  assert.ok(reachesNode(workflow, (path) => scripts[path] ?? null));
  assert.ok(!reachesNode(workflow, () => null), "an unreadable script claims nothing");
});

test("setup-node itself is not mistaken for running Node", () => {
  const workflow = `
      - uses: actions/setup-node@abc # v4
        with:
          node-version-file: .nvmrc
      - run: cargo test --workspace
`;

  assert.ok(!reachesNode(workflow, () => null));
});

test("a cargo step that merely mentions a node in prose does not count", () => {
  assert.ok(!reachesNode("      - name: Check every node in the graph\n", () => null));
});
