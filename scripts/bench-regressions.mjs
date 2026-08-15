#!/usr/bin/env node

// Decides which criterion comparisons are real regressions.
//
// Criterion tests each benchmark independently. This workspace measures 39
// points -- thirteen policies at three sizes, plus the churn groups -- so at a
// five percent significance level roughly two of them are expected to be
// labelled changed on every run where nothing changed at all. A gate that cries
// wolf twice a run is a gate people learn to ignore, which is worse than not
// having one.
//
// Two filters, both required to call something a regression.
//
// Holm-Bonferroni controls the family-wise error rate across all comparisons in
// the run: the chance of *any* false alarm stays at the stated level, rather
// than that being the per-comparison rate. Holm rather than plain Bonferroni
// because it is uniformly more powerful and no less valid, so it costs nothing
// to prefer.
//
// An effect-size floor rejects changes too small to act on. Round robin picks
// in about five nanoseconds; a statistically impeccable one percent regression
// there is fifty picoseconds, which is not a fact about the code. Significance
// answers whether a difference is real, not whether it matters, and only the
// second question should stop a merge.

import { readdir, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const criterion = join(root, "target", "criterion");

const familyWiseAlpha = Number(process.env.POISE_BENCH_ALPHA ?? "0.05");
const minimumEffect = Number(process.env.POISE_BENCH_MIN_EFFECT ?? "0.05");

/** Standard normal CDF via Abramowitz and Stegun 7.1.26. */
export function normalCdf(z) {
  const sign = z < 0 ? -1 : 1;
  const x = Math.abs(z) / Math.SQRT2;
  const t = 1 / (1 + 0.3275911 * x);
  const poly =
    t *
    (0.254829592 +
      t *
        (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
  const erf = 1 - poly * Math.exp(-x * x);
  return 0.5 * (1 + sign * erf);
}

/** Two-sided p-value for a relative change against its standard error. */
export function twoSidedP(pointEstimate, standardError) {
  if (!(standardError > 0)) {
    // No dispersion means no evidence of a difference, not certainty of one.
    return 1;
  }
  const z = Math.abs(pointEstimate / standardError);
  return Math.max(0, Math.min(1, 2 * (1 - normalCdf(z))));
}

/**
 * Holm-Bonferroni adjusted p-values, in the order the comparisons were given.
 *
 * Sort ascending, scale the i-th by the number of hypotheses still untested,
 * and enforce monotonicity so an adjusted value never falls below one that
 * precedes it. Comparing the result against alpha then controls the family-wise
 * error rate.
 */
export function holmAdjust(pValues) {
  const ordered = pValues
    .map((value, index) => ({ value, index }))
    .sort((left, right) => left.value - right.value);

  const adjusted = new Array(pValues.length);
  let running = 0;
  ordered.forEach(({ value, index }, rank) => {
    const scaled = (pValues.length - rank) * value;
    running = Math.max(running, Math.min(1, scaled));
    adjusted[index] = running;
  });
  return adjusted;
}

/** Collects every comparison criterion recorded against a baseline. */
async function collect(directory, trail = []) {
  const entries = await readdir(directory, { withFileTypes: true });
  const found = [];

  for (const entry of entries) {
    if (!entry.isDirectory()) {
      continue;
    }
    if (entry.name === "change") {
      const raw = await readFile(join(directory, "change", "estimates.json"), "utf8");
      const estimates = JSON.parse(raw);
      found.push({
        name: trail.join("/"),
        pointEstimate: estimates.mean.point_estimate,
        standardError: estimates.mean.standard_error,
      });
      continue;
    }
    if (entry.name === "base" || entry.name === "new" || entry.name === "report") {
      continue;
    }
    found.push(...(await collect(join(directory, entry.name), [...trail, entry.name])));
  }

  return found;
}

function percent(value) {
  return `${(value * 100).toFixed(2)}%`;
}

async function main() {
  let comparisons;
  try {
    comparisons = await collect(criterion);
  } catch {
    console.error(
      "no criterion results found; run scripts/bench.sh --save-baseline NAME " +
        "and then scripts/bench.sh --baseline NAME",
    );
    process.exit(2);
  }

  if (comparisons.length === 0) {
    console.error(
      "criterion recorded no comparisons; the second run needs --baseline NAME " +
        "so there is something to compare against",
    );
    process.exit(2);
  }

  const pValues = comparisons.map((comparison) =>
    twoSidedP(comparison.pointEstimate, comparison.standardError),
  );
  const adjusted = holmAdjust(pValues);

  const regressions = [];
  const improvements = [];
  let significantButSmall = 0;

  comparisons.forEach((comparison, index) => {
    const significant = adjusted[index] <= familyWiseAlpha;
    const large = Math.abs(comparison.pointEstimate) >= minimumEffect;
    if (!significant) {
      return;
    }
    if (!large) {
      significantButSmall += 1;
      return;
    }
    const record = { ...comparison, adjusted: adjusted[index] };
    if (comparison.pointEstimate > 0) {
      regressions.push(record);
    } else {
      improvements.push(record);
    }
  });

  console.log(
    `${comparisons.length} comparisons, family-wise alpha ${familyWiseAlpha}, ` +
      `effect floor ${percent(minimumEffect)}`,
  );

  for (const improvement of improvements) {
    console.log(
      `  improved  ${improvement.name}: ${percent(improvement.pointEstimate)} ` +
        `(adjusted p ${improvement.adjusted.toExponential(2)})`,
    );
  }
  for (const regression of regressions) {
    console.log(
      `  REGRESSED ${regression.name}: ${percent(regression.pointEstimate)} ` +
        `(adjusted p ${regression.adjusted.toExponential(2)})`,
    );
  }
  if (significantButSmall > 0) {
    console.log(
      `  ${significantButSmall} comparison(s) significant but below the effect ` +
        "floor, which is the regime where a real difference is not a meaningful one",
    );
  }

  if (regressions.length > 0) {
    console.error(`${regressions.length} regression(s) survived both filters`);
    process.exit(1);
  }
  console.log("no regression survived both filters");
}

if (import.meta.url === `file://${process.argv[1]}`) {
  await main();
}
