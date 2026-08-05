#!/usr/bin/env node

import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, extname, join, normalize, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const docs = join(root, "docs");
const summaryPath = join(docs, "SUMMARY.md");
const failures = [];

function proseOnly(source) {
  let fence = null;
  return source
    .split("\n")
    .map((line) => {
      const marker = line.match(/^\s*(`{3,}|~{3,})/)?.[1];
      if (marker && fence === null) {
        fence = marker[0];
        return "";
      }
      if (marker && marker[0] === fence) {
        fence = null;
        return "";
      }
      return fence === null ? line : "";
    })
    .join("\n");
}

const summary = await readFile(summaryPath, "utf8");
const chapterLinks = [...summary.matchAll(/\[[^\]]+\]\(([^)]+\.md(?:#[^)]+)?)\)/g)]
  .map((match) => match[1].split("#", 1)[0]);
const chapters = [...new Set(chapterLinks)];

if (chapters.length !== chapterLinks.length) {
  failures.push("SUMMARY.md contains a chapter more than once");
}

const markdownFiles = (await readdir(docs))
  .filter((name) => extname(name) === ".md" && name !== "SUMMARY.md")
  .sort();

for (const file of markdownFiles) {
  if (!chapters.includes(file)) {
    failures.push(`docs/${file} is not listed in SUMMARY.md`);
  }
}

for (const chapter of chapters) {
  const chapterPath = resolve(docs, chapter);
  if (!chapterPath.startsWith(`${docs}/`)) {
    failures.push(`chapter escapes docs/: ${chapter}`);
    continue;
  }

  let source;
  try {
    source = await readFile(chapterPath, "utf8");
  } catch {
    failures.push(`missing chapter: docs/${chapter}`);
    continue;
  }

  const prose = proseOnly(source);
  const headings = prose.match(/^# .+$/gm) ?? [];
  if (headings.length !== 1) {
    failures.push(`docs/${chapter} must contain exactly one level-one heading (found ${headings.length})`);
  }

  const links = [...prose.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)].map((match) => match[1]);
  for (const rawTarget of links) {
    const target = rawTarget.trim().replace(/^<|>$/g, "").split("#", 1)[0];
    if (!target || /^(?:[a-z][a-z+.-]*:|\/\/)/i.test(target)) continue;

    const decoded = decodeURIComponent(target);
    const candidate = resolve(dirname(chapterPath), decoded);
    try {
      const info = await stat(candidate);
      if (!info.isFile() && !info.isDirectory()) {
        failures.push(`docs/${chapter} links to unsupported target: ${target}`);
      }
    } catch {
      failures.push(`docs/${chapter} has a broken local link: ${target}`);
    }
  }
}

if (failures.length > 0) {
  console.error("Book validation failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log(`Book validation passed: ${chapters.length} chapters, ${markdownFiles.length} Markdown sources.`);
}
