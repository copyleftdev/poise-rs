# Testing and verification

Poise treats behavioral laws as part of its public API. Ordinary unit tests
cover examples, edge cases, reference vectors, distributions, concurrency, and
transactional failures. The `poise-core` integration suite adds generated,
shrinking property tests that exercise only public APIs.

Run the standard property suite with:

```console
cargo test -p poise-core --test property_selection
cargo test -p poise-core --test property_affinity
cargo test -p poise-core --test property_topology
```

Each property runs 256 generated cases by default. The suite currently checks:

- every policy either selects an eligible in-bounds endpoint or returns the
  precise empty/ineligible error;
- least-loaded minimality, round-robin cycle coverage, and exact smooth
  weighted-round-robin conservation;
- exact seeded replay for stochastic, priority, and locality policies;
- affinity identity independence from candidate order;
- rendezvous addition and removal minimal-disruption laws;
- equivalence of weighted and unweighted rendezvous at equal weights;
- priority health/panic eligibility and fully healthy failover suppression;
- locality decisions preserving selected priority and topology metadata.

For a longer local or scheduled soak, set the Poise-specific case count:

```console
POISE_PROPTEST_CASES=10000 cargo test -p poise-core --test property_selection
```

Shrinking is bounded at 10,000 iterations. Minimized failure seeds are written
beside each integration test in a `.regressions` file. Those files are
intentional repository artifacts: commit newly discovered seeds so every later
run replays the failure before exploring novel inputs.

The property suite uses Proptest 1.11 with only its `std` feature. That release
declares Rust 1.85 compatibility, matching the workspace MSRV. It is an exact
development-dependency pin so a test-only dependency update cannot silently
raise the supported compiler version.

## Distribution sensitivity

A test asserting that counts fall inside a hand-picked band is a hypothesis test
whose operating point nobody wrote down. The in-module distribution tests use
bands about six standard deviations wide, which is the right shape — a false
alarm is then about one in a billion, so the suite does not flake — but at their
sample sizes six standard deviations is also five percent of the expected count,
so a sampler biased by four percent passes.

`tests/distribution_power.rs` keeps the threshold and fixes the other half. The
sample size is derived from the deviation the test claims to detect, and each
assertion first checks that its own design is powerful enough to see that
deviation.

Sizing carries an explicit power target. Placing the claimed deviation exactly
on the rejection boundary detects it only about half the time, because the
estimate is centred on the boundary and falls short as often as it clears it, so
the sample must place that deviation `Z + Z_power` standard deviations out:

```text
n >= margin * (Z * sqrt(p(1-p)) + Z_power * sqrt(q(1-q)))^2 / (q - p)^2
```

where `q = p(1 + relative)` is the deviation being detected. `Z` is 6.8 and
`Z_power` is 1.645, a ninety-five percent one-sided detection target, which
costs about 1.5 times the samples a fifty percent target would.

Each term carries the variance belonging to its own hypothesis: the null's under
the rejection threshold, the alternative's under the power term. Using the
null's for both is tidier, and lands below the required count.

Sizing is checked against an exact one-proportion power calculation performed
outside this repository. That check is what found the tidier expression sitting
0.15% to 0.66% short — always short, never over, which is how a stated
sensitivity decays into a slogan. The size now carries a two percent margin,
wider than the disagreement between standard forms, and a test holds it between
the requirement and a tenth above it so the margin cannot grow to cover a real
error. An underpowered test fails
as loudly as a biased sampler, so sensitivity cannot decay quietly as the code
around it changes. Two further tests check the checker, one supplying a sample
too small and one a bias just past the claimed resolution.

Randomized policies are swept over a fixed set of seeds rather than one. A
single seed makes a distribution test a regression test on one draw: a sampler
biased for every seed but that one passes forever. Fixing the *set* keeps the
run reproducible, which the mutation gate requires, while sampling several
points of the seed space.

These tests are `#[ignore]` by default and run in release:

```console
cargo test --release -p poise-core --test distribution_power -- --ignored
```

The samples run to millions. That costs tens of milliseconds optimized and
about three seconds unoptimized, and the mutation campaign runs the unoptimized
suite once per mutant, where three seconds would become half an hour of
campaign time. The split keeps the sensitivity without charging every mutant
for it.

## Claims about the tree

Every count in the prose is a claim with an expiry date nobody wrote down. This
repository has shipped several past theirs: ten Loom models when there were
thirteen, 240 test entry points when there were 259, 642 mutation sites when
there were 705, and a performance chapter still calling criterion baselines a
roadmap item after they had landed. Each was accurate when written, which is the
whole difficulty — prose drifts silently, and stays readable while doing it.

`scripts/check-docs-drift.mjs` counts the derivable claims from the source and
fails when the prose disagrees. A claim whose pattern no longer matches is also
a failure, and so is a claim stated twice: a checker that quietly stops checking
is worse than no checker, and a duplicated claim leaves a stale copy behind
every correction.

It also resolves every local link and every repository path the prose names,
skipping the ones git ignores. Generated output is named legitimately — the book
builds to `site/book/`, which the prose says and a clean checkout does not
contain — so checking its existence would make the gate answer differently on a
developer machine than in CI, which is the one thing a gate must not do. The
number skipped is printed rather than passed over quietly, since a gate that
skips silently is indistinguishable from one that has stopped checking.
Links in `docs/` are already validated by the book check, but README and the
root documents are outside it, and fenced code is stripped before that check
runs — so a chapter telling a reader to run a script is precisely the claim
nothing verified. That is the claim most likely to rot after a rename, and the
one a reader hits first.

Facts stated in more than one place get checked against each other rather than
against nothing. The required CI checks are one fact with three copies — the
workflow defines the job names, `scripts/protect-main.sh` requires them, and
[releasing](releasing.md) lists them — and renaming a job desynchronises the
other two silently, at which point a required context that no longer reports
blocks every merge or a job quietly stops gating. The MSRV is one fact with
five: the manifest declares it, a badge shows it, the README says it, the test
matrix pins it, and protection names the resulting job. The manifest is the only
copy a compiler enforces, so it is the source and the rest are compared to it.

Some claims cannot be derived by a script that does not run the campaign
producing them. The recorded mutation results are checked for internal
consistency instead — a breakdown that no longer sums to its own total is drift
arithmetic can catch. Counts fixed at compile time, such as the metrics
cardinality, are asserted in the crate that owns them rather than scraped, since
that is where a change to them happens.

Property testing is one verification layer, not a replacement for mutation,
fuzz, model-checking, benchmark, simulation, and coverage gates. The mutation
policy and reproducible `poise-core` campaign are documented in
[mutation testing](mutation-testing.md). Resource-bounded fuzz targets and the
exhaustive in-flight concurrency model are covered in
[fuzzing and concurrency models](fuzzing.md).
