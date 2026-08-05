# Architecture

Load balancing is not one algorithm. It is a control loop with distinct stages:

```text
discovery -> membership -> eligibility -> selection -> dispatch -> feedback
                              ^                           |
                              +-- health / outliers <-----+
```

Poise keeps those stages separate. This prevents a policy from silently owning
connections, spawning a runtime, deciding retry behavior, or inventing health
semantics on behalf of its caller.

## Layer boundaries

### Membership

Membership turns DNS, static configuration, Kubernetes watches, xDS, or a
custom source into versioned backend snapshots. Updates must be atomic from a
selector's perspective. Removed backends may enter a draining state before
their resources are retired.

`poise-discovery` implements this boundary with a single-writer `Directory` and
immutable snapshots. Change batches can be staged transactionally. A removal
first changes the member to `Draining`; an adapter calls `finish_drain` only
after outstanding work releases its shared backend handles. Snapshots are
published through an atomic pointer swap, and strictly increasing revisions
prevent stale state from replacing newer state.

With its optional `discovery` feature, `poise-tower` reconciles those snapshots
into a live service pool. Stable key plus unchanged backend allocation retains
the service, load tracker, and Tower readiness reservation. A new backend
allocation for the same key creates a new service generation. Builds are staged
before commit, duplicate identities and stale revisions are rejected, and pool
ordering follows the coherent snapshot.

Snapshot readers also expose a runtime-neutral, multi-subscriber stream. Each
subscriber owns an independent revision cursor and waker. Publications are
coalesced state rather than a lossless event log, so a slow control loop moves
directly to the newest coherent snapshot. Dropping the single publisher wakes
subscribers and terminates their streams after the final state is observed.

### Eligibility

Eligibility combines administrative state, active health, passive failure
signals, circuit state, capacity, locality constraints, and draining policy.
Every exclusion should carry a machine-readable reason. The policy core starts
with the portable states `Ready`, `Draining`, and `Unavailable`.

`poise-health` adds a generic `HealthSignal` boundary and a composable
`HealthChecked` candidate. Its passive circuit uses permits to make half-open
probe limits race-safe. Consecutive failures open the circuit, elapsed cooldown
moves it to half-open, successful probes restore it, and late results from an
older circuit epoch cannot mutate newer state.

Active health is also executor-neutral. The library reserves at most one due
probe, while the caller chooses the timer, runtime, protocol, timeout, and
response classifier. Explicit healthy or unhealthy completion advances
consecutive-result thresholds; cancellation merely reschedules. Generation
tokens prevent results from superseded probes from changing current health.
Clock-aware reservation and completion methods let adapters keep simulated or
runtime-specific monotonic time in one domain.

`poise-tokio` supplies that adapter for Tokio. It waits for due reservations,
enforces an optional timeout, and makes timeout classification explicit:
unhealthy changes threshold state, while cancellation does not. The reservation
is finalized even when the runner future is dropped. The adapter also exposes
allocation-free futures for discovery snapshot streams and race-free waits for
a minimum revision.

### Selection

A selection policy sees a coherent candidate slice and returns an index. It
does not clone a backend or dispatch work. This supports zero-copy callers,
borrowed snapshots, custom candidate types, deterministic tests, and policies
that need request context.

Policy families planned for the core include:

- cyclic: round robin, smooth weighted round robin;
- stochastic: random, weighted random, power of two choices;
- load-aware: least loaded, least requests, peak EWMA;
- affinity: rendezvous, weighted rendezvous, precomputed ring hash, Maglev, and
  bounded-load rendezvous spillover;
- topology-aware: weighted priority failover with overprovisioning and explicit
  panic behavior, followed by health-adjusted locality weighting and endpoint
  capacity selection;
- adaptive: choice policies driven by measured cost and capacity.

### Dispatch

Adapters translate a selected index into work on a protocol or service stack.
Readiness polling, connection pooling, queueing, timeouts, cancellation, and
backpressure remain adapter concerns.

`poise-tower` implements this boundary without changing the core policy trait.
Each endpoint retains its own Tower readiness reservation. `poll_ready` polls
all eligible idle services, and `call` lets the policy choose only among those
that are actually ready before consuming exactly one reservation. Pending
services do not block healthy peers. A service that fails readiness is
quarantined until explicitly reset, while an observer hook preserves isolated
errors that do not fail the aggregate pool.

The response future holds an endpoint-specific load guard. Any returned result,
including a service error, is a completed attempt; dropping a pending future is
cancellation. This makes in-flight and peak-EWMA policies reflect real dispatch
lifetime without requiring a particular executor. Request-context projectors
also allow affinity policies to borrow a routing key without allocation.

Physical retirement drops the pool's endpoint handle but cannot invalidate an
already returned Tower future, which owns its service future and load guard.
Stream polling and runtime-specific wakeup loops remain outside the reconciler;
callers may either drive synchronization explicitly or use
`StreamingDiscoveryBalance`, which polls discovery before service readiness.
Its bounded per-poll update budget prevents a continuously changing control
plane from starving the Tower task, and it supports last-known-good or
fail-closed behavior when the publisher ends.

### Feedback

Attempts produce structured outcomes: latency, cancellation, overload,
transport failure, and application result. Trackers feed these into load
estimators, passive health, outlier ejection, and observability without coupling
the core policy trait to an async runtime.

The core load trackers use RAII completion guards, so cancellation and panic
unwinding cannot strand an in-flight count. Load-aware policies compare sampled
immutable metrics rather than imposing `Ord` on concurrently changing tracker
handles. `PeakEwma` combines decaying observed latency with current concurrency
while remaining independent of any async executor.

Bounded-load rendezvous treats `InFlight` counts as additive cluster load.
Weighted rendezvous supplies affinity order, while a prospective weighted-share
bound spills hot keys to the next ranked backend with room. Every load is
sampled once into reusable policy scratch. The returned detailed decision keeps
the unconstrained owner visible, and atomic admission limits remain separate
because concurrent selectors can race after observing the same snapshot.

Attempt results are classified as success, failure, overload, or cancellation.
Rolling outcome windows ignore cancellation, retain bounded history, and expose
a configurable penalty metric with a minimum-sample gate to limit cold-start
noise. Explicit overload can carry more weight than an ordinary failure.

Group-relative outlier analysis establishes a success-rate baseline only from
hosts with enough samples. It returns deterministic, worst-first candidate
indices below a configurable standard-deviation threshold, bounded by both a
maximum ejection percentage and a minimum healthy group size. Detection is pure:
the control plane decides how long to eject a host and which health signal to
change.

`poise-observe` consumes these portable decisions and outcomes without changing
their ownership. `ObservedPolicy` delegates transparently and reports the exact
selection result. `Attempt` records explicit completion or drop-based
cancellation with elapsed monotonic time. A cloneable metrics recorder uses
fixed enum dimensions, a fixed latency histogram, and relaxed atomic counters;
backend identity, request keys, endpoint indices, policy names, and errors are
never metric labels. Optional tracing emits structured spans and events, while
an optional Tower adapter counts isolated readiness failures without retaining
their error values.

## Planned workspace

| Crate | Responsibility |
| --- | --- |
| `poise-core` | Candidate model, selection traits, policies, deterministic utilities |
| `poise-discovery` | Versioned snapshots, atomic publication, graceful draining |
| `poise-health` | Active/passive health, circuit state, outlier detection |
| `poise-tokio` | Optional Tokio timing and async discovery conveniences |
| `poise-tower` | Tower `Service` and `Layer` adapters |
| `poise-observe` | Fixed-cardinality metrics, tracing, decision observation |
| `poise-sim` | Workload simulation, policy comparison, regression fixtures |

These are architectural boundaries, not a promise to publish a crate for every
row. A crate is split only when it produces a real dependency or compatibility
boundary.

## Compatibility principles

- Core public types avoid runtime and protocol dependencies.
- Randomized policies accept reproducible seeds and injectable RNGs.
- Keyed policies use documented, deterministic hashing by default.
- Policy errors distinguish an empty set from a non-empty but ineligible set.
- Adding a backend must not mutate caller-owned backend values.
- Policy implementations document complexity, allocation, and membership-change
  behavior.
- New policy APIs require simulation evidence and adversarial tests.

## Explicit non-goals

Poise is not an HTTP client, reverse proxy, service mesh, DNS resolver, or
orchestrator SDK. It should make those systems easier to build without forcing
their protocols or runtimes into the core.
