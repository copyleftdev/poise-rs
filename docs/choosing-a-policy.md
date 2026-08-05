# Choose a policy

Policy choice begins with the behavior your traffic needs to preserve. “Evenly
distributed” is not a complete requirement: distribution, affinity, load
response, membership churn, topology, and state all pull in different
directions.

## Decision table

| Requirement | Start with | Why | Watch for |
| --- | --- | --- | --- |
| Stable cycling over a small pool | `RoundRobin` | Deterministic, allocation-free, no RNG | Slice reorder changes the cycle |
| Capacity-proportional cycling | `SmoothWeightedRoundRobin` | Exact long-run integer ratios without bursts | State is keyed by identity |
| Simple probabilistic spread | `Random` | Uniform among eligible candidates | No capacity or load signal |
| Capacity-proportional spread | `WeightedRandom` | Honors weight under frequent membership change | Two scans and checked weight sum |
| React to a live load signal | `LeastLoaded` | Chooses the measured global minimum | Every candidate is sampled |
| Approximate load balance | `PowerOfTwoChoices` | Avoids always choosing the global minimum | Current implementation still scans to sample |
| Sticky keys, equal capacity | `Rendezvous` | Minimal disruption with no lookup table | O(n) hash scoring per pick |
| Sticky keys, unequal capacity | `WeightedRendezvous` | Capacity-aware minimal disruption | Fixed-point logarithmic scoring |
| Sticky keys with load escape | `BoundedLoadRendezvous` | Preserves owner until its prospective bound is full | Requires a coherent additive load view |
| Familiar continuum semantics | `RingHash` | Configurable virtual-node ring | Membership rebuild and point budget |
| Fast repeated keyed lookup | `Maglev` | Cached O(1) table lookup | Fixed table size and rebuild cost |
| Region or tier failover | `PriorityWeightedRandom` | Explicit spillover and panic behavior | Health percentage interpretation |
| Priority plus locality | `LocalityWeightedRandom` | Selects scope before endpoint capacity | Metadata consistency per locality |

This table identifies a starting point. Validate the exact candidate count,
membership churn, weight distribution, and request-key distribution you expect.

## Start from the invariant

### “Every eligible backend should take turns”

Use `RoundRobin` when configured weights and live load do not matter. It is
easy to reason about and exposes ordering mistakes quickly.

Use `SmoothWeightedRoundRobin` when an endpoint with weight 5 should receive
exactly five selections for every one selection received by weight 1 over a
complete cycle, without sending the five selections as one burst.

### “Capacity should shape traffic”

Use `WeightedRandom` when candidate membership changes frequently and an O(n)
scan is acceptable. It builds no alias table and therefore has no rebuild
lifecycle.

Weights are ratios, not request limits. A 4:1 configuration does not prevent the
larger backend from being overloaded, and it does not reserve four concrete
permits.

### “Current work should shape traffic”

Use `LeastLoaded` when the load metric is meaningful across every candidate and
sampling all candidates is affordable. Equal-load ties rotate to avoid
permanently favoring the first slice entry.

Use `PowerOfTwoChoices` when comparing two samples better matches the desired
control behavior. The policy needs a `LoadMetric`, not a particular tracker.
`InFlight`, `PeakEwma`, and application-owned metrics can all participate.

Load selection is observational. If capacity must be enforced atomically, pair
selection with `InFlight::try_acquire` or another admission boundary.

### “A key should stay with its owner”

Use rendezvous hashing for direct, table-free affinity. Removing a backend can
only move keys owned by that backend; adding a backend can only preserve the
existing owner or move the key to the new backend.

Use ring hash when integrations require continuum behavior or tunable points.
Use Maglev when a stable membership set serves enough keyed lookups to amortize
table construction.

Use bounded-load rendezvous when hot keys must spill away from their affinity
owner. It preserves the unconstrained owner in decision metadata so operators
can distinguish ordinary ownership from capacity-driven spillover.

### “Failure domains should shape traffic”

Priority and locality are scope selection, not decorations on endpoint weight.
The policy first decides which priority is usable, then which locality receives
traffic, then which endpoint wins inside that locality.

Do not flatten priority, locality, and endpoint capacity into one number. That
loses the ability to explain failover and creates surprising behavior during
partial outages.

## Questions to answer before production

1. What identity remains stable across discovery updates?
2. Does candidate order remain stable, or can it be reconstructed by identity?
3. Are weights capacity ratios, commercial allocations, or emergency knobs?
4. Is the load signal comparable across processes and generations?
5. Does a request need affinity, and what disruption is acceptable on churn?
6. Which health states may panic routing revive, if any?
7. Is selection followed by an atomic capacity reservation?
8. What decision metadata must reach traces or incident logs?

If these answers are unclear, choose the simplest policy and instrument it
before adding affinity or adaptive behavior.

## Common mismatches

- **Round robin over unstable ordering:** membership reorder becomes traffic
  movement even when identities did not change.
- **Affinity for mutable keys:** a key derived from timestamps, random IDs, or
  noncanonical encodings defeats stickiness.
- **Weights as hard limits:** weights influence proportion; they do not enforce
  admission.
- **Peak EWMA without cancellation:** abandoned work appears completed and
  distorts the estimator.
- **Panic as universal revival:** draining and operator opt-out must remain
  excluded even during a health panic.
- **Metrics as policy state:** asynchronous exporter state is not a coherent
  hot-path load signal.

The focused contract chapters document each family’s exact arithmetic and
membership behavior.
