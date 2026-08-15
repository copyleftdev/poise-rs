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
n >= (Z + Z_power)^2 * (1 - p) / (relative^2 * p)
```

`Z` is 6.8 and `Z_power` is 1.645, a ninety-five percent one-sided detection
target, which costs about 1.5 times the samples that a fifty percent target
would. An underpowered test fails
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

Property testing is one verification layer, not a replacement for mutation,
fuzz, model-checking, benchmark, simulation, and coverage gates. The mutation
policy and reproducible `poise-core` campaign are documented in
[mutation testing](mutation-testing.md). Resource-bounded fuzz targets and the
exhaustive in-flight concurrency model are covered in
[fuzzing and concurrency models](fuzzing.md).
