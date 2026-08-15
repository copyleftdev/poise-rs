import assert from "node:assert/strict";
import test from "node:test";

import {
  checkSum,
  ignoredPaths,
  localLinks,
  referencedPaths,
  countLoomModels,
  countProptestLaws,
  countTestAttributes,
  spell,
  statedValue,
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
