# Maglev

`Maglev` implements the lookup-table construction introduced in Google's
Maglev network load balancer. Each eligible backend receives a deterministic
permutation of a fixed table. Construction visits those permutations in turns,
so backend slot counts differ by at most one. A request hashes to one slot and
therefore needs one array access after the live candidate slice has been
validated.

The implementation follows the original paper's unweighted algorithm. Backend
weights are intentionally ignored: a weight-only update neither rebuilds the
table nor changes an assignment. Applications that need weighted affinity
should use `WeightedRendezvous` or `RingHash`; weighted Maglev should be added
only with a separately specified and tested construction.

## Configuration

`MaglevConfig` requires a prime table size. Prime sizing makes every skip in
`1..table_size` coprime to the table size, so each backend permutation covers
every slot. The default is 65,537 entries and the hard maximum is 5,000,011,
matching Envoy's documented operational bounds. Reconciliation also rejects
more eligible backends than table entries rather than committing a table in
which some backend is unreachable.

A larger table improves balance and generally reduces disruption, at the cost
of proportionally more memory and rebuild work. The table contains `usize`
candidate indices, in addition to `O(n)` member and construction state.

## Cache and reconciliation

`reconcile` compares eligible identity, slice index, and order with the
committed membership. Eligibility, addition, removal, or reorder rebuilds the
table. The policy stores candidate indices, so a reorder must rebuild; canonical
identity ordering ensures that normal hash inputs retain the same winning
identity afterward. If both independent ordering hashes collide, slice index is
the documented deterministic fallback.

Construction happens in temporary allocations. Duplicate eligible identities,
excess member count, and allocation failure return `PickError` without replacing
the last committed table or advancing its generation. `reset` clears the cached
state while retaining configuration and the hash builder.

The ordinary `Policy::pick` method reconciles before lookup. An unchanged pick
therefore performs `O(n)` exact validation, followed by `O(1)` request lookup,
with no allocation. Control planes that already know when membership changes
can call `reconcile` explicitly and inspect `MaglevUpdate`, but safe picks still
validate the borrowed slice so stale candidate indices cannot escape.

## Churn semantics

Maglev is minimally disruptive in a statistical sense, not the strict
remove-only remapping sense of rendezvous hashing. Adding or removing a backend
usually preserves most assignments, while a small number of keys owned by
unchanged backends can also move during table reconstruction. Use rendezvous
hashing when that stronger invariant matters more than constant-time lookup.

The default `FnvBuildHasher` plus Poise's stable avalanche finalizer makes table
construction and request replay deterministic across processes using the same
candidate identities and configuration. Caller-provided hash builders define
their own compatibility behavior and should produce identical hashes on every
balancer expected to share assignments.

## References

- [Maglev: A Fast and Reliable Software Network Load Balancer](https://www.usenix.org/system/files/conference/nsdi16/nsdi16-paper-eisenbud.pdf), Eisenbud et al., NSDI 2016.
- [Envoy Maglev load-balancing policy](https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/load_balancing_policies/maglev/v3/maglev.proto).
