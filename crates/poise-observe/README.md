# poise-observe

Bounded-cardinality observability for Poise load balancing.

The crate provides lock-free process metrics, transparent policy decorators,
RAII attempt observation, and composable observers. Optional features add
structured `tracing` spans/events and an adapter for Tower readiness failures.

The default feature set is empty. Enable `tracing` for structured spans and
events, and `tower` for the readiness-error observer adapter.

Metric dimensions are closed enums. Backend identities, request keys, endpoint
indices, policy names, and error strings are never metric labels. Diagnostic
indices may appear in tracing events, where they do not create persistent
metric series.

```rust
use poise_core::{Backend, Policy, policy::RoundRobin};
use poise_observe::{DecisionKind, Metrics, ObservedPolicy};

let candidates = [Backend::new("west"), Backend::new("east")];
let metrics = Metrics::new();
let mut policy = ObservedPolicy::new(RoundRobin::new(), metrics.clone());

let selected = policy.pick(&candidates, &()).unwrap();
assert_eq!(selected.index(), 0);
assert_eq!(metrics.snapshot().decisions(DecisionKind::Selected), 1);
```
