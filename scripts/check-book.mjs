#!/usr/bin/env node

import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const root = resolve(import.meta.dirname, "..");
const docs = join(root, "docs");

/** Removes fenced code so examples cannot masquerade as document structure. */
export function proseOnly(source) {
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

/** Returns the anchor IDs mdBook derives from a Markdown document's headings. */
export function markdownHeadingIds(source) {
  const ids = new Set();
  const occurrences = new Map();
  const prose = proseOnly(source);
  const headings = prose.matchAll(/^#{1,6}[ \t]+(.+?)[ \t]*#*[ \t]*$/gm);

  for (const match of headings) {
    const label = match[1]
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/<[^>]*>/g, "")
      .replace(/[`*_~]/g, "");
    const base = label
      .toLocaleLowerCase("en-US")
      .replace(/[^\p{Letter}\p{Number}\s_-]/gu, "")
      .trim()
      .replace(/\s+/g, "-");
    if (!base) continue;

    const occurrence = occurrences.get(base) ?? 0;
    occurrences.set(base, occurrence + 1);
    ids.add(occurrence === 0 ? base : `${base}-${occurrence}`);
  }

  return ids;
}

/** Returns explicit IDs from HTML, or derived heading IDs from Markdown. */
export function documentIds(source, extension) {
  if ([".md", ".markdown"].includes(extension)) {
    return markdownHeadingIds(source);
  }
  if ([".html", ".htm"].includes(extension)) {
    return new Set([...source.matchAll(/\sid=(?:"([^"]+)"|'([^']+)')/g)]
      .map((match) => match[1] ?? match[2]));
  }
  return null;
}

/** Validates the chapter graph and every resolvable local document link. */
export async function checkBook(docsDirectory = docs) {
  const failures = [];
  const summaryPath = join(docsDirectory, "SUMMARY.md");
  const summary = await readFile(summaryPath, "utf8");
  const chapterLinks = [...summary.matchAll(/\[[^\]]+\]\(([^)]+\.md(?:#[^)]+)?)\)/g)]
    .map((match) => match[1].split("#", 1)[0]);
  const chapters = [...new Set(chapterLinks)];

  if (chapters.length !== chapterLinks.length) {
    failures.push("SUMMARY.md contains a chapter more than once");
  }

  const markdownFiles = (await readdir(docsDirectory))
    .filter((name) => extname(name) === ".md" && name !== "SUMMARY.md")
    .sort();

  for (const file of markdownFiles) {
    if (!chapters.includes(file)) {
      failures.push(`docs/${file} is not listed in SUMMARY.md`);
    }
  }

  const idCache = new Map();
  for (const chapter of chapters) {
    const chapterPath = resolve(docsDirectory, chapter);
    if (!chapterPath.startsWith(`${docsDirectory}/`)) {
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
      const target = rawTarget.trim().replace(/^<|>$/g, "");
      if (!target || /^(?:[a-z][a-z+.-]*:|\/\/)/i.test(target)) continue;

      const fragmentAt = target.indexOf("#");
      const rawPath = fragmentAt >= 0 ? target.slice(0, fragmentAt) : target;
      const rawFragment = fragmentAt >= 0 ? target.slice(fragmentAt + 1) : "";
      let decodedPath;
      let fragment;
      try {
        decodedPath = decodeURIComponent(rawPath);
        fragment = decodeURIComponent(rawFragment);
      } catch {
        failures.push(`docs/${chapter} has invalid URL encoding: ${target}`);
        continue;
      }

      const candidate = decodedPath ? resolve(dirname(chapterPath), decodedPath) : chapterPath;
      try {
        const info = await stat(candidate);
        if (!info.isFile() && !info.isDirectory()) {
          failures.push(`docs/${chapter} links to unsupported target: ${target}`);
          continue;
        }

        if (fragment && info.isFile()) {
          let ids = idCache.get(candidate);
          if (ids === undefined) {
            const targetSource = candidate === chapterPath ? source : await readFile(candidate, "utf8");
            ids = documentIds(targetSource, extname(candidate).toLowerCase());
            idCache.set(candidate, ids);
          }
          if (ids !== null && !ids.has(fragment)) {
            failures.push(`docs/${chapter} has a broken local fragment: ${target}`);
          }
        }
      } catch {
        failures.push(`docs/${chapter} has a broken local link: ${target}`);
      }
    }
  }

  return { failures, chapterCount: chapters.length, sourceCount: markdownFiles.length };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const result = await checkBook();
  if (result.failures.length > 0) {
    console.error("Book validation failed:");
    for (const failure of result.failures) console.error(`- ${failure}`);
    process.exitCode = 1;
  } else {
    console.log(`Book validation passed: ${result.chapterCount} chapters, ${result.sourceCount} Markdown sources.`);
  }
}
