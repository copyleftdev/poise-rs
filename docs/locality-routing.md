# Locality-weighted routing

`LocalityWeightedRandom` composes failover priority, locality preference, and
endpoint capacity without collapsing their independent meanings. Every
decision follows this hierarchy:

```text
priority -> health-adjusted locality -> weighted endpoint
```

Priority is selected first using the same availability, overprovisioning, and
panic contract as `PriorityWeightedRandom`. Only localities in that selected
priority participate in the second stage. A very large weight in a failover
priority therefore cannot steal traffic from a fully provisioned primary.

## Candidate metadata

Applications can implement `LocalityCandidate` for their own coherent snapshot
type, or compose the built-in wrappers:

```rust
use poise_core::{Backend, Weight, policy::{Localized, Prioritized}};

let endpoint = Localized::new(
    Prioritized::new(Backend::new("api-west-1"), 0),
    "us-west-2",
)
.with_locality_weight(Weight::new(3)?);
# Ok::<(), poise_core::InvalidWeight>(())
```

There are two deliberately separate weights:

- `LocalityCandidate::locality_weight` expresses the control plane's desired
  share between localities at one priority.
- `Candidate::weight` expresses endpoint capacity and divides traffic only
  inside the chosen locality.

All endpoints in one `(priority, locality)` group must advertise the same
locality weight. `InconsistentTopology` rejects contradictory snapshots rather
than making their result depend on candidate order.

## Health and spillover

For locality `L`, Poise computes fixed-point millionth-share availability from
weighted endpoint capacity:

```text
availability(L) = min(100%, overprovisioning × eligible_weight(L) / total_weight(L))
effective_weight(L) = configured_locality_weight(L) × availability(L)
traffic(L) = effective_weight(L) / sum(effective_weight)
```

The default overprovisioning factor is 140 percent. A locality therefore keeps
its full configured share while at least 5/7 of its weighted capacity remains
eligible. Beyond that point, its shortfall spills proportionally across the
other selected-priority localities.

For example, locality X with configured weight 1 and 50 percent available
capacity has effective weight 70. Fully available locality Y with configured
weight 2 has effective weight 200. Their resulting shares are approximately
26 and 74 percent. Once a locality is selected, endpoint weights select among
only its eligible members.

Eligibility, priority membership, and panic eligibility are sampled once per
decision by the shared priority engine. Locality health uses that exact sample;
it never re-reads a changing health signal. In `UseAll` panic mode,
panic-eligible endpoints form the selectable capacity. `FailClosed` retains
the priority policy's `PanicRejected` behavior. Draining endpoints are not
configured capacity by default.

Millionth-share arithmetic is deterministic and integer-only. A nonempty
locality receives a minimum effective availability unit if quantization would
otherwise round it to zero. Weight accumulation is checked and returns
`WeightOverflow` rather than wrapping.

## Scope

This policy implements explicit control-plane locality weights. It does not
infer that the caller's zone must always be preferred, because that requires a
request-origin distribution and zone-aware routing policy. Callers can express
strict regional failover with priorities, proportional cross-region routing
with locality weights, or combine both.

Calculation retains `O(n)` scratch and takes `O(n log n)` time for priority and
locality grouping plus `O(n)` endpoint selection. Stable candidate high-water
marks allocate nothing after warmup. `seeded` and `seeded_with` provide exact
replay for the same candidate order and metadata.

The hierarchy and health-adjusted weighting model are informed by Envoy's
[locality-weighted load balancing](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/locality_weight),
[priority routing](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/priority),
and the distinction between locality and endpoint weights in its
[endpoint API](https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/endpoint/v3/endpoint_components.proto).
Envoy's separate
[zone-aware policy](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/zone_aware.html)
illustrates why implicit caller-local preference is a different primitive.
