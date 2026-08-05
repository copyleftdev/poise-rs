# Ring-hash contract

`RingHash` implements weighted consistent hashing with a cached virtual-node
table. It follows the ring and successor lookup model introduced by
[Karger et al.](https://doi.org/10.1145/258533.258660) and the weighted
virtual-node practice described by
[Envoy's ring-hash documentation](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/load_balancers.html#ring-hash).

## Construction and lookup

Each eligible candidate receives:

```text
normalized_weight × virtual_nodes_per_weight
```

points. `normalized_weight` is the configured positive weight divided by the
greatest common divisor of all eligible weights. Thus `[1, 3]` and `[100, 300]`
build identical rings with identical memory use.

Virtual points are hashed into the `u64` space and sorted. A request hashes into
that same space and selects the first point at or clockwise from its position,
wrapping to the first point after the end of the ring.

For `r` virtual points, rebuild cost is `O(r log r)` time and `O(r)` memory.
Every safe `Policy::pick` first validates the candidate slice in `O(n)`, then
performs an `O(log r)` binary search. An unchanged lookup allocates nothing and
does not rebuild the table. `generation()` and `RingUpdate` expose rebuilds for
tests and control-plane diagnostics.

## Reconciliation

The cache identity includes every eligible candidate's exact identity, weight,
and slice index. Changes to membership, order, eligibility, or weight trigger a
staged rebuild. The old table is committed only after the replacement has been
fully validated, allocated, populated, and sorted.

Reconciliation rejects:

- duplicate eligible identity with `PickError::DuplicateIdentity`;
- point-count overflow, configured-cap violations, and allocation failure with
  `PickError::StateCapacityExceeded`.

Neither failure replaces the last valid table. A subsequent valid candidate
slice can continue using or replace it normally.

## Capacity and distribution

`RingHashConfig` specifies virtual nodes per normalized unit weight and a hard
maximum point count. The default is 128 points per unit and 1,048,576 total
points. A larger table generally approximates desired weight ratios more
closely, at the cost of rebuild time and memory.

The cap is checked before table allocation. It is a normal selection error, not
a reason to silently reduce resolution or omit a low-weight backend.

## Disruption guarantees

When the eligible set's weight greatest-common-divisor remains unchanged,
adding or removing one backend leaves every other backend's virtual points
unchanged. Only keys won by the added backend or previously owned by the removed
backend move.

When a membership or weight change alters that divisor, normalization can add
or remove points for otherwise unchanged backends. This preserves scale
invariance of relative weights but can cause more churn than the ideal
adjacent-only ring update. Applications that require the strictest churn bound
should use coprime/canonically scaled weights or `WeightedRendezvous`, whose
scores do not use set-wide normalization.

Reordering a unique-identity slice rebuilds stored indices but preserves winning
identities, except in the pathological case of complete hash collisions.

## Hashing and collisions

Separate domains hash candidate identities, virtual-node replicas, and request
keys. Each result passes through Poise's stable `mix64` avalanche finalizer.
The default FNV builder is reproducible but not collision-resistant; use
`with_hasher` for adversarial inputs.

Point ordering is total even if hashes collide, so lookup never panics. The
fallback order is position, owner hash, replica number, then current slice
index. Consequently complete collisions remain safe and deterministic for one
slice, but reordering that slice may change the winner. A suitable hash builder
makes this edge negligible; the behavior is defined rather than hidden.
