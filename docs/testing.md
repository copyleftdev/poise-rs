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

Property testing is one verification layer, not a replacement for mutation,
fuzz, model-checking, benchmark, simulation, and coverage gates. The mutation
policy and reproducible `poise-core` campaign are documented in
[mutation testing](mutation-testing.md). Resource-bounded fuzz targets and the
exhaustive in-flight concurrency model are covered in
[fuzzing and concurrency models](fuzzing.md).
