# Observability contract

`poise-observe` turns portable policy decisions and attempt outcomes into
telemetry without changing selection or dispatch behavior. Its default build
has no tracing, metrics-facade, runtime, or Tower dependency.

## Cardinality budget

The built-in `Metrics` recorder has no caller-defined labels. Its storage shape
is fixed at compile time:

| Signal | Dimension | Maximum counters |
| --- | --- | ---: |
| Policy decisions | `DecisionKind` | 10 |
| Backend attempts | `AttemptKind` | 5 |
| Readiness failures | none | 1 |
| Attempt latency | fixed cumulative bounds plus `+Inf` | 11 |
| Attempt latency sum | none | 1 |

The complete recorder therefore owns 28 saturating counters, regardless of
backend count, traffic shape, or input diversity.

Backend identity, endpoint index, request or affinity key, policy name, route,
error text, and discovery source are never metric dimensions. This remains true
even when those values have millions of distinct values. Applications may add
their own bounded labels while exporting a `MetricsSnapshot`, but doing so is
outside the library's safety guarantee.

Counters saturate at `u64::MAX`. Recording uses relaxed atomic operations. A
snapshot reads those atomics independently, so under concurrent updates it
represents a narrow interval rather than a globally transactional instant.

`DecisionKind::ALL` and `AttemptKind::ALL` let exporters enumerate every stable
dimension. `ATTEMPT_LATENCY_BOUNDS` and `ATTEMPT_LATENCY_BUCKET_COUNT` expose
the complete cumulative histogram shape.

## Policy decisions

`ObservedPolicy` implements the same `Policy` contract as its inner policy. It
returns the original `Selection` or `PickError` unchanged and emits exactly one
`DecisionEvent` afterward.

```rust
use poise_core::{Backend, Policy, policy::LeastLoaded};
use poise_observe::{DecisionKind, Metrics, ObservedPolicy};

let metrics = Metrics::new();
let mut policy = ObservedPolicy::new(LeastLoaded::new(), metrics.clone());
let candidates = [Backend::new("a"), Backend::new("b")];

let _selection = policy.pick(&candidates, &()).unwrap();
assert_eq!(metrics.snapshot().decisions(DecisionKind::Selected), 1);
```

Observers receive candidate count and selected slice index as diagnostic
context. `Metrics` discards both values as labels; `TracingObserver` can include
them as event fields.

## Attempt lifetime

`Attempt` owns an observer and a monotonic start time. Calling `success`,
`failure`, `overloaded`, `cancel`, or `complete` records exactly one result.
Dropping an unfinished guard records cancellation, covering task cancellation,
early returns, and panic unwinding.

```rust
use poise_observe::{Attempt, AttemptKind, Metrics};

let metrics = Metrics::new();
let attempt = Attempt::new(metrics.clone());
// Dispatch protocol-specific work.
attempt.success();

assert_eq!(metrics.snapshot().attempts(AttemptKind::Success), 1);
```

Application code retains responsibility for translating protocol and
application results into Poise's portable outcome classes.

## Tracing

The optional `tracing` feature adds:

- `TracingObserver`, which emits decision and attempt events at `DEBUG` and
  readiness failures at `WARN`;
- `TracedPolicy`, which wraps each synchronous policy call in a
  `poise.policy.pick` span and records its fixed decision classification.

Targets are `poise::decision`, `poise::attempt`, and `poise::readiness`.
Tracing records contain numeric indices and fixed classifications, but exclude
backend IDs, request keys, policy names, and errors by default.

Use `Fanout::new(metrics, TracingObserver)` when the same records should reach
both built-in sinks. Wrapping a `TracedPolicy` in an `ObservedPolicy` is also
valid: the former supplies the timing span and the latter supplies metrics or
completion events.

## Tower readiness

The optional `tower` feature adds `TowerObserver`, which implements
`poise_tower::ObserveReadinessError`. Install it through
`Balance::with_readiness_observer`:

```rust,no_run
# use poise_core::{Backend, policy::RoundRobin};
# use poise_observe::{Metrics, TowerObserver};
# use poise_tower::{Balance, Endpoint};
# use std::{convert::Infallible, future};
# use tower::service_fn;
# fn echo(value: u32) -> future::Ready<Result<u32, Infallible>> {
#     future::ready(Ok(value))
# }
let metrics = Metrics::new();
let endpoints = vec![Endpoint::new(Backend::new("a"), service_fn(echo))];
let balance = Balance::new(endpoints, RoundRobin::new())
    .with_readiness_observer(TowerObserver::new(metrics.clone()));
# let _ = balance;
```

The adapter counts the error and discards endpoint and error values for metric
purposes. Applications that need full error diagnostics can install a Tower
closure that forwards to the adapter and separately logs or handles the
borrowed error.
