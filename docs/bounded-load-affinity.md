# Bounded-load affinity

`BoundedLoadRendezvous` combines capacity-proportional rendezvous hashing with
live concurrent-load bounds. Weighted rendezvous establishes the stable
preference order for a request key. The first candidate in that order with
spare prospective capacity wins, so idle routing is exactly
`WeightedRendezvous` while hot affinity owners spill to deterministic peers.

This is the request-routing form of consistent hashing with bounded loads, not
a stateful implementation of the paper's complete balls-to-bins allocation
algorithm. The original algorithm owns the global assignment set and
rebalances it after updates. Poise observes a borrowed snapshot of concurrent
work and chooses one destination; it therefore does not claim the paper's
global movement bounds.

## Capacity invariant

For eligible candidate `i`, the policy computes:

```text
capacity_i = ceil(
    balance_factor_percent * (total_current_load + 1) * weight_i
    / (100 * total_eligible_weight)
)
```

The `+ 1` accounts for the request being selected. A candidate is available
when its sampled load is strictly less than this capacity, which means its load
after one successful admission is no greater than the reported bound. With a
factor of at least 100 percent and positive `Weight` values, at least one
eligible candidate has room in every representable snapshot.

The default balance factor is 150 percent. A factor of 100 gives the tightest
bound; larger factors preserve affinity more often but allow more imbalance.
Envoy documents 120–200 percent as a typical operational range for its related
hash-balance feature.

Weights affect both affinity distribution and load capacity. A backend with
weight three receives three times the ideal request share of a unit-weight
backend. The formula uses checked integer arithmetic and exact ceiling division;
it does not route based on floating-point capacity comparisons.

## Load contract

Candidate loads must implement `LoadMetric<Metric = u64>` and represent current
concurrent work. `InFlight` is the intended built-in tracker. `PeakEwma` is not
accepted because a latency-times-concurrency score is not an additive request
count and has no meaningful cluster average for this bound.

Each eligible load is sampled exactly once per decision. The policy retains an
`O(n)` sample buffer, so it allocates while growing to a new high-water
candidate count and then reuses that memory. `shrink_to_fit` lets a control
plane explicitly return retained scratch memory. Selection and sampling are
`O(n)`.

The snapshot invariant is not an atomic admission guarantee. Concurrent
selectors can observe the same spare slot and race after selection. Use
`InFlight::with_limit` or another atomic admission mechanism when exceeding a
process-local hard limit must fail rather than briefly overshoot.

## Decisions and errors

`decide` returns `BoundedLoadDecision`, containing:

- `affinity`: the unconstrained weighted-rendezvous owner;
- `selection`: the candidate selected after applying bounds;
- `spilled`: whether those candidates differ;
- the selected candidate's sampled load and prospective capacity.

The ordinary `Policy::pick` implementation returns only `selection`. Systems
that measure affinity spillover should call `decide` and record their own
bounded-cardinality event; request keys and backend identities should not become
metric labels.

Empty and wholly ineligible slices keep the standard `PickError` distinction.
Eligible weight accumulation can return `WeightOverflow`, load accumulation can
return `LoadOverflow`, and sampling-buffer growth can return
`StateCapacityExceeded`. No backend is selected from a partial sample.

## Churn behavior

While every candidate is below capacity, routing has the exact weighted
rendezvous guarantees: changing one backend does not move a key directly
between two otherwise unchanged backends. Overload deliberately relaxes sticky
routing. A key moves to its highest-ranked candidate with room and returns to
its affinity owner when later samples place that owner below capacity.

This ranking avoids the cascading neighbor overflow associated with linear
probing on a ring. It also makes the result independent of candidate slice order
for unique identities, weights, and corresponding load samples.

## References

- [Consistent Hashing with Bounded Loads](https://research.google/pubs/consistent-hashing-with-bounded-loads/), Mirrokni, Thorup, and Zadimoghaddam, SODA 2018.
- [Google Research algorithm overview](https://research.google/blog/consistent-hashing-with-bounded-loads/).
- [Envoy ring-hash balance-factor contract](https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/load_balancing_policies/ring_hash/v3/ring_hash.proto).
