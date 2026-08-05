# Discovery and Tower

Enable `poise-tower`'s optional `discovery` feature to reconcile
`poise-discovery` snapshots into a live `DiscoveryBalance`. The feature is off
by default, preserving the base Tower adapter's minimal dependency graph.

```toml
[dependencies]
poise-tower = { version = "0.1", features = ["discovery"] }
```

## Service generations

Discovery identity and backend allocation have separate meanings:

- the stable key identifies the logical member across revisions;
- the shared backend allocation identifies one configuration generation.

When both are unchanged, reconciliation retains the complete `Endpoint`: Tower
service, readiness reservation, failure quarantine, and dispatch load. A drain
or other membership-only transition therefore excludes new traffic without
discarding live state.

An upsert creates a new backend allocation even if its value compares equal.
The reconciler treats that as an intentional generation change and asks the
`EndpointFactory` for a replacement service and load tracker. The old endpoint
is dropped only when the new revision commits.

## Transactional application

For every newer snapshot, reconciliation:

1. validates the revision and uniqueness of live and snapshot identities;
2. classifies members as retained, added, or rebuilt;
3. stages every required service generation through the factory;
4. commits endpoints in snapshot order and advances the applied revision.

If validation or any factory call fails, the live endpoint set, ordering,
readiness, load, and applied revision remain unchanged. Successfully staged
services are dropped. External effects performed inside a factory cannot be
rolled back, so factories should avoid publishing a service elsewhere before
returning it.

An equal revision is an idempotent no-op. An older revision returns
`ReconcileError::StaleRevision`. `ReconcileReport` counts retained, added,
rebuilt, removed, and draining members for control-plane observability.

## Factory choices

An `EndpointFactory` returns both a Tower service and a `LoadTracker`. Closures
that return `(service, tracker)` implement it directly. Wrap a service-only
closure with `in_flight_factory` to create a fresh unbounded `InFlight` tracker
for every service generation.

```rust
# use std::convert::Infallible;
# use poise_core::{Backend, policy::RoundRobin};
# use poise_discovery::Directory;
# use poise_tower::{DiscoveryBalance, in_flight_factory};
let mut directory = Directory::new();
directory.upsert("west", Backend::new("http://west"))?;

let factory = in_flight_factory(|_key: &&str, backend: &Backend<&str>| {
    // A real factory would construct a protocol client from this value.
    Ok::<_, Infallible>(backend.id().to_string())
});
let mut balance = DiscoveryBalance::new(RoundRobin::new(), factory);
let report = balance.apply_snapshot(&directory.snapshot())?;
assert_eq!(report.added(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Draining and retirement

A draining snapshot preserves the endpoint but makes its `Discovered`
candidate ineligible immediately. Existing response futures remain independent
of that eligibility transition. After the application observes no outstanding
work and calls `Directory::finish_drain`, the next snapshot removes the endpoint
from the pool. A response future that was already returned still owns its load
guard and remains valid even after physical retirement.

## Driving synchronization

`DiscoveryBalance::sync` performs one atomic load from a `SnapshotReader` and
applies it. It does not poll a stream or spawn a task. An application may call
it from configuration callbacks, a control-plane loop, or a runtime-specific
watch task. Keeping this primitive synchronous makes its transaction and error
semantics deterministic while leaving wakeups and retry policy to the adapter
that owns the runtime.

For event-driven operation, `SnapshotReader::subscribe` returns a
runtime-neutral `SnapshotStream`. Its first item is the current snapshot;
`SnapshotReader::changes` starts after the current revision instead. Each
subscriber has its own waker and cursor. If several revisions arrive between
polls, only the newest snapshot is yielded—membership snapshots represent
current state, not an event log. The stream ends after its publisher is dropped
and its final revision has been observed.

`StreamingDiscoveryBalance` combines that subscription with a
`DiscoveryBalance`. Every `poll_ready` first reconciles visible discovery state,
then polls endpoint readiness. The same task waker is registered with both
sources, so no driver task or Tokio dependency is required.

`StreamingConfig` bounds the number of applications per readiness poll. If the
budget is exhausted, the wrapper self-wakes and yields `Pending`, preventing a
busy control plane from starving other work. Publisher closure defaults to
last-known-good service; `StreamEndPolicy::FailClosed` instead rejects readiness
after the final snapshot. Reconciliation failures are returned as
`StreamingError::Reconcile` while leaving the previous pool intact. Since the
failed snapshot has been observed, a later poll can continue serving the last
good pool or apply a newer publication.
