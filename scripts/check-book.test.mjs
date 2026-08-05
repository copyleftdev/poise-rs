import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { checkBook, documentIds, markdownHeadingIds, proseOnly } from "./check-book.mjs";

test("fenced examples do not create document headings", () => {
  const source = "# Real title\n\n```rust\n# hidden_example()\n```\n\n## Visible section\n";

  assert.equal(proseOnly(source).includes("hidden_example"), false);
  assert.deepEqual([...markdownHeadingIds(source)], ["real-title", "visible-section"]);
});

test("Markdown heading IDs preserve code names and disambiguate repeats", () => {
  const source = "# Load and feedback\n\n## The `LoadMetric` boundary\n\n## Retry budget\n\n## Retry budget\n";

  assert.deepEqual([...markdownHeadingIds(source)], [
    "load-and-feedback",
    "the-loadmetric-boundary",
    "retry-budget",
    "retry-budget-1",
  ]);
});

test("document IDs expose exact Markdown and HTML fragment targets", () => {
  const markdown = documentIds("# API map\n\n## `poise-core`\n", ".md");
  const html = documentIds("<h2 id=\"selection\">Selection</h2><div id='dispatch'></div>", ".html");

  assert.equal(markdown.has("poise-core"), true);
  assert.equal(markdown.has("missing"), false);
  assert.deepEqual([...html], ["selection", "dispatch"]);
  assert.equal(documentIds("plain text", ".txt"), null);
});

test("book validation rejects missing same-document and cross-document fragments", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "poise-book-check-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(join(directory, "SUMMARY.md"), "# Summary\n\n[First](first.md)\n\n- [Second](second.md)\n");
  await writeFile(
    join(directory, "first.md"),
    "# First\n\n## Present\n\n[Same document](#missing)\n\n[Other document](second.md#absent)\n",
  );
  await writeFile(join(directory, "second.md"), "# Second\n\n## Available\n");

  const result = await checkBook(directory);

  assert.deepEqual(result.failures, [
    "docs/first.md has a broken local fragment: #missing",
    "docs/first.md has a broken local fragment: second.md#absent",
  ]);
});
