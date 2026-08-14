# Roadmap

## Foundation

- [x] Candidate, status, weight, selection, and policy contracts.
- [x] Round robin, random, weighted random, least loaded, P2C, and rendezvous.
- [x] Deterministic tests, generated policy invariants, and rustdoc examples.
- [x] Criterion benchmarks and recorded baselines for `poise-core` selection
  and membership-change cost. Dispatch, health, and discovery remain unmeasured.
- Naming, licensing, security policy, MSRV policy, and governance before
  publication.

## Dynamic systems

- [x] Immutable versioned snapshots with keyed backend identity.
- [x] Atomic snapshot publication and graceful draining.
- [x] Smooth weighted round robin with identity-keyed state migration.
- [x] Load trackers for in-flight requests and peak-EWMA latency.
- [x] Rolling success-rate and overload penalties.
- [x] Passive health and circuit breaking with bounded recovery probes.
- [x] Executor-neutral active probe scheduling with threshold transitions.
- [x] Group-relative success-rate outlier detection with safety caps.

## Ecosystem integration

- [x] Tower adapter with retained readiness, load tracking, and cancellation.
- [x] Transactional static snapshot-to-Tower reconciliation.
- [x] Runtime-neutral coalescing snapshot streams and Tower reconciliation.
- [x] Optional Tokio active-health and discovery conveniences without making
  Tokio a core dependency.
- [x] `tracing` spans and metrics with bounded-cardinality defaults.

## Advanced policies

- [x] Weighted rendezvous with proportional capacity and minimal reweight churn.
- [x] Bounded weighted virtual-node ring hash with transactional rebuilds.
- [x] Prime-sized, transactionally rebuilt Maglev lookup tables.
- [x] Weighted rendezvous affinity with prospective concurrent-load bounds.
- [x] Weighted priority tiers, overprovisioned failover, and panic thresholds.
- [x] Health-adjusted locality weighting and cross-locality spillover.
- Retry and hedge selection that excludes already-attempted backends.
- Adaptive concurrency and capacity-aware routing.

## Verification bar

Every stable policy should have:

- unit, property, and deterministic replay tests;
- distribution and disruption tests where applicable;
- criterion benchmarks across small and large backend sets;
- documented time, memory, and allocation complexity;
- behavior documented for empty sets, ineligible sets, ties, overflow,
  membership churn, and seeded randomness;
- model-based or simulation comparison against a reference implementation.
