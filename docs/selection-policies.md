# Selection policies

This chapter covers the non-keyed policy surface in `poise-core`. Every policy
implements the same contract:

```rust
pub trait Policy<C, Context: ?Sized = ()> {
    fn pick(
        &mut self,
        candidates: &[C],
        context: &Context,
    ) -> Result<Selection, PickError>;
}
```

## Shared invariants

For every general policy:

- a returned index is inside the supplied slice;
- the selected candidate reports `is_eligible() == true`;
- an empty slice returns `PickError::Empty`;
- a non-empty slice with no eligible candidates returns
  `PickError::NoEligibleCandidates`;
- the policy does not clone, mutate, or dispatch the candidate;
- configuration and arithmetic failure remain explicit errors.

The returned `Selection` intentionally carries an index. Membership ownership,
backend borrowing, service lookup, and dispatch stay with the caller.

## Round robin

`RoundRobin` scans from a cursor and advances past the selected index.
Ineligible candidates are skipped.

| Property | Value |
| --- | --- |
| Pick time | O(n) worst case |
| Extra memory | O(1) |
| State | Slice cursor |
| Determinism | Exact for a stable slice |
| Membership sensitivity | Reordering changes the cycle |

An arbitrary initial cursor is reduced modulo the current slice length.
Applications that reconcile membership should preserve ordering when possible.

## Smooth weighted round robin

`SmoothWeightedRoundRobin<Key>` maintains identity-keyed current weights.
Across a complete cycle, each eligible candidate receives exactly its configured
integer share, while high-weight selections are spread through the cycle.

State follows identity rather than slice position. Ineligible and absent
identities are pruned. Duplicate eligible identities are rejected because two
state entries cannot safely represent one logical backend.

Use this policy when exact long-run ratios matter more than independent random
draws.

## Uniform random

`Random` uses reservoir sampling to select uniformly among eligible candidates
without allocating an intermediate list.

| Property | Value |
| --- | --- |
| Pick time | O(n) |
| Extra memory | O(1) |
| Draws | One bounded draw per eligible candidate, including the first |
| Reproducibility | `Random::seeded` or caller RNG |

Reservoir sampling means sparse eligibility does not require a preliminary
count or temporary vector.

## Weighted random

`WeightedRandom` performs two scans. The first checks and sums eligible
weights into `u64`; the second resolves one ticket.

The checked sum can return `PickError::WeightOverflow`. A configuration that
cannot be represented is rejected instead of silently biasing the
distribution.

No alias table is cached. This favors frequently changing candidate sets and
keeps rebuild behavior out of the policy.

## Least loaded

`LeastLoaded` samples every eligible candidate’s `LoadMetric` and chooses the
smallest value. Ties rotate in slice order using an internal cursor.

```rust
use poise_core::{Backend, Policy, policy::LeastLoaded};

let candidates = [
    Backend::new("a").with_load(3_u64),
    Backend::new("b").with_load(1_u64),
];
let mut policy = LeastLoaded::new();

assert_eq!(policy.pick(&candidates, &())?.index(), 1);
# Ok::<(), poise_core::PickError>(())
```

The metric is read during selection and may change immediately afterward.
Atomic admission remains a separate concern.

## Power of two choices

`PowerOfTwoChoices` reservoir-samples two eligible candidates, compares their
load metrics, and chooses the smaller. A single eligible candidate wins
directly; equal-load ties use the RNG.

The current implementation scans the slice to sample without allocation. Its
advantage is the selection behavior—not an O(1) candidate lookup claim.

## State ownership

Policy instances are mutable because RNG state, cursors, cached tables, or
identity maps can advance. Decide deliberately how instances are shared:

- one instance per worker creates independent sequences;
- a mutex around one instance creates global sequencing and contention;
- deterministic sharding by worker preserves replay within each shard;
- reconstructing an instance resets its state.

Poise does not hide synchronization inside the `Policy` trait. The application
chooses the concurrency boundary appropriate for its request path.

## Custom candidates and the policy boundary

Implement `Candidate` for a borrowed snapshot view when copying into
`Backend` would obscure ownership.

Downstream policy implementation is not currently a supported extension point:
although `Policy` is public, constructing a successful `Selection` is reserved
to `poise-core`. Use the in-repository policies and their public context and
candidate extension traits. A future checked policy extension must preserve the
same eligible, in-bounds result contract before downstream implementations are
documented as supported.
