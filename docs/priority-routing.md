# Priority routing and panic

`PriorityWeightedRandom` routes across ordered failover groups. Priority `0`
receives traffic first, then priority `1`, and so on. A lower priority receives
only traffic that higher priorities cannot cover according to their weighted
availability and the configured overprovisioning factor.

The primitive deliberately performs endpoint selection itself using weighted
random choice. Its name makes that behavior explicit; it does not pretend to
wrap an arbitrary policy through an eligibility mask that the core `Policy`
contract cannot express. Priority planning and endpoint sampling share one
coherent snapshot, and the detailed decision reports both the selected priority
and whether normal or panic eligibility was used.

## Candidate metadata

Applications can implement `PriorityCandidate` for their own candidate type or
wrap any existing `Candidate` in `Prioritized<C>`. Lower `u32` priority values
are preferred; gaps are allowed and carry no semantic weight.

Three sampled predicates remain distinct:

- priority membership: configured capacity used as the availability
  denominator;
- normal eligibility: healthy capacity used during ordinary routing;
- panic eligibility: members that use-all panic may revive.

The built-in wrapper excludes `Draining` candidates from membership and panic.
It allows other unavailable members in panic by default. Set
`with_panic_eligibility(false)` for a candidate whose hard capacity, policy, or
administrative exclusion must never be bypassed. This is intentionally explicit:
panic must not silently override a circuit breaker or admission limit merely
because both happen to make `is_eligible` false.

## Availability and spillover

For each priority, the policy computes fixed-point millionth-share values:

```text
raw_availability = eligible_weight / configured_member_weight
effective_availability = min(
    100%,
    raw_availability * overprovisioning_factor
)
```

The default overprovisioning factor is 140 percent. Thus a priority remains able
to carry all traffic until its weighted availability drops below roughly
71.4 percent. Factors below 100 are rejected.

When combined effective availability reaches 100 percent, priorities consume
traffic capacity in ascending order. For example, effective availability of
70 percent at priority 0 leaves 30 percent for priority 1 even if priority 1
could handle more. When combined availability is below 100 percent, remaining
shares are normalized rather than leaving an accidental random-selection gap.

Candidate `Weight` participates in all three relevant places: availability
accounting, global-panic priority share, and endpoint selection within the
chosen priority. Accumulation is checked and returns `WeightOverflow` rather
than wrapping.

## Panic

Panic is considered only when combined effective availability across all
priorities is below 100 percent. If lower priorities provide enough capacity,
an unhealthy primary spills traffic normally and does not panic. This avoids
reviving unhealthy endpoints while a sound failover remains available.

Within a globally underprovisioned snapshot, a priority enters panic when its
raw weighted availability is strictly below `panic_threshold_percent`. The
default is 50 percent; zero disables panic.

`PanicMode::UseAll` chooses among explicitly panic-eligible members. Draining
and opted-out members remain excluded. `PanicMode::FailClosed` returns
`PickError::PanicRejected` when traffic lands on a panicking priority, making
intentional load shedding distinguishable from an empty or ordinarily
ineligible cluster. If every priority has zero effective availability, use-all
panic distributes traffic across priorities in proportion to their
panic-eligible weights.

## Complexity and consistency

Membership and all eligibility predicates are sampled once per decision. The
policy retains member and group buffers, allocating only when a new high-water
candidate count exceeds their capacity. Priority aggregation sorts temporary
groups, so calculation is `O(n log n)` time with `O(n)` retained memory;
endpoint choice is `O(n)`.

`PriorityDecision` exposes the candidate `Selection`, numeric priority, and
`PriorityMode::{Healthy, Panic}`. The regular `Policy::pick` method returns only
the selection. Seeded constructors provide deterministic replay for tests and
simulations; ordinary construction seeds from the process random source.

## References

- [Envoy priority panic-threshold behavior](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/panic_threshold.html).
- [Envoy overprovisioning factor](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/overprovisioning.html).
- [Envoy locality and priority failover example](https://www.envoyproxy.io/docs/envoy/latest/start/sandboxes/locality-load-balancing.html).
