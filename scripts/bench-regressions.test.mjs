import assert from "node:assert/strict";
import test from "node:test";

import { holmAdjust, normalCdf, twoSidedP } from "./bench-regressions.mjs";

test("the normal CDF matches known quantiles", () => {
  assert.ok(Math.abs(normalCdf(0) - 0.5) < 1e-6);
  assert.ok(Math.abs(normalCdf(1.959_964) - 0.975) < 1e-4);
  assert.ok(Math.abs(normalCdf(-1.959_964) - 0.025) < 1e-4);
  assert.ok(Math.abs(normalCdf(2.575_829) - 0.995) < 1e-4);
});

test("a two-sigma difference is about a five percent two-sided p-value", () => {
  const p = twoSidedP(1.959_964, 1);
  assert.ok(Math.abs(p - 0.05) < 1e-3, `expected about 0.05, got ${p}`);
});

test("no dispersion is treated as no evidence rather than certainty", () => {
  // A zero standard error would otherwise divide to infinity and report every
  // benchmark as a certain regression, which is the opposite of what an absent
  // measurement means.
  assert.equal(twoSidedP(0.5, 0), 1);
  assert.equal(twoSidedP(0, 0), 1);
});

test("Holm scales the smallest p-value by the number of comparisons", () => {
  // The classic worked example: four hypotheses, the smallest scaled by four.
  const adjusted = holmAdjust([0.01, 0.02, 0.03, 0.04]);

  assert.ok(Math.abs(adjusted[0] - 0.04) < 1e-12);
  assert.ok(Math.abs(adjusted[1] - 0.06) < 1e-12);
  assert.ok(Math.abs(adjusted[2] - 0.06) < 1e-12);
  assert.ok(Math.abs(adjusted[3] - 0.06) < 1e-12);
});

test("Holm adjusted values never decrease", () => {
  // Monotonicity is what stops a later, weaker comparison being reported as
  // stronger than an earlier one purely through the scaling factor.
  const raw = [0.001, 0.6, 0.02, 0.9, 0.04];
  const adjusted = holmAdjust(raw);
  const byRawOrder = raw
    .map((value, index) => ({ value, adjusted: adjusted[index] }))
    .sort((left, right) => left.value - right.value)
    .map((entry) => entry.adjusted);

  for (let index = 1; index < byRawOrder.length; index += 1) {
    assert.ok(
      byRawOrder[index] >= byRawOrder[index - 1],
      `adjusted values decreased: ${byRawOrder}`,
    );
  }
});

test("Holm is never anti-conservative relative to the raw p-value", () => {
  const raw = [0.001, 0.01, 0.03, 0.2, 0.5];
  const adjusted = holmAdjust(raw);
  raw.forEach((value, index) => {
    assert.ok(adjusted[index] >= value, `adjustment lowered ${value}`);
  });
});

test("Holm is no more conservative than Bonferroni", () => {
  // Uniformly more powerful is the reason to prefer it, so a case where it were
  // stricter would mean the implementation is wrong rather than cautious.
  const raw = [0.001, 0.01, 0.03, 0.2, 0.5];
  const adjusted = holmAdjust(raw);
  raw.forEach((value, index) => {
    assert.ok(
      adjusted[index] <= Math.min(1, raw.length * value) + 1e-12,
      `adjustment exceeded Bonferroni at ${value}`,
    );
  });
});

test("a family of pure noise raises no alarm at the stated level", () => {
  // Thirty-nine comparisons of unchanged code, one of which lands at p = 0.03 by
  // chance. Uncorrected that is a regression; corrected it is a Tuesday.
  const raw = [0.03, ...Array.from({ length: 38 }, (_, index) => 0.2 + index * 0.02)];
  const adjusted = holmAdjust(raw);

  assert.ok(raw[0] < 0.05, "the example needs an uncorrected alarm to suppress");
  assert.ok(
    adjusted.every((value) => value > 0.05),
    "correction failed to suppress a chance alarm",
  );
});

test("a genuine outlier still survives correction", () => {
  // The correction must not be so blunt that a real regression is lost with the
  // noise, which would trade a chatty gate for a useless one.
  const raw = [1e-9, ...Array.from({ length: 38 }, (_, index) => 0.2 + index * 0.02)];
  const adjusted = holmAdjust(raw);

  assert.ok(adjusted[0] <= 0.05, `real regression was suppressed at ${adjusted[0]}`);
});
