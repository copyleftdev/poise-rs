# Public API map

This map is an orientation aid, not a substitute for rustdoc. It shows where
public concepts live and which feature boundaries introduce optional
dependencies.

## `poise-core`

### Candidate model

| API | Role |
| --- | --- |
| `Candidate` | Borrowed policy view of identity, weight, load, status, eligibility |
| `Backend` | Concrete candidate with application data |
| `Status` | Ready, draining, or unavailable administrative state |
| `Weight` | Validated nonzero integer capacity ratio |
| `Selection` | Validated policy result wrapper around a slice index |
| `PickError` | Typed no-candidate and arithmetic failures |

### Policy contract

| API | Role |
| --- | --- |
| `Policy<C, Context>` | Mutable selection operation |
| `PolicyExt::choose` | Convenience lookup returning a candidate borrow |
| `Random`, `WeightedRandom` | Stateless stochastic spread |
| `RoundRobin`, `SmoothWeightedRoundRobin` | Stateful cyclic spread |
| `LeastLoaded`, `PowerOfTwoChoices` | Load-aware choice |
| `Rendezvous`, `WeightedRendezvous` | Table-free affinity |
| `BoundedLoadRendezvous` | Affinity with prospective capacity spillover |
| `RingHash`, `Maglev` | Cached affinity structures |
| `PriorityWeightedRandom` | Priority, spillover, and panic |
| `LocalityWeightedRandom` | Priority plus locality health weighting |

### Feedback and load

| API | Role |
| --- | --- |
| `Outcome` | Success, failure, overload, cancellation |
| `LoadMetric` | Comparable load measurement |
| `InFlight`, `InFlightGuard` | Atomic concurrency accounting |
| `PeakEwma`, `PeakEwmaGuard` | Latency-and-concurrency estimator |
| `LoadScore` | Ordered validated score representation |

## `poise-discovery`

| API | Role |
| --- | --- |
| `Directory` | Single-writer transactional membership state |
| `Change`, `Effect`, `Applied` | Reconciliation input and report |
| `Revision` | Monotonically increasing snapshot version |
| `Discovered`, `Membership` | Candidate wrapper and lifecycle |
| `Snapshot<T>` | Immutable revisioned state |
| `snapshot_channel` | Single-publisher, multi-reader channel |
| `SnapshotPublisher` | Atomic publication boundary |
| `SnapshotReader`, `SnapshotStream` | Coalescing subscribers |

## `poise-health`

| API | Role |
| --- | --- |
| `HealthSignal`, `HealthChecked` | Composable candidate eligibility |
| `PassiveHealth`, `CircuitPermit` | Epoch-safe passive circuit |
| `CircuitConfig`, `CircuitSnapshot` | Circuit configuration and observation |
| `ActiveHealth`, `ActiveProbe` | Executor-neutral scheduled health state |
| `ActiveHealthConfig`, `ActiveSnapshot` | Probe thresholds and observation |
| `OutcomeWindow`, `OutcomeStats` | Bounded recent result history |
| `PenaltyScore` | Load-compatible recent-failure penalty |
| `OutlierDetector`, `OutlierReport` | Pure group-relative analysis |

## `poise-tower`

| API | Role |
| --- | --- |
| `Endpoint<C, S, L>` | Candidate, Tower service, and generation load tracker |
| `Balance` | Readiness-aware policy-driven Tower service |
| `BalanceError` | Selection, readiness, tracker, and service failures |
| `RequestContext`, `UseRequest` | Borrowed request projection for affinity |
| `LoadTracker`, `LoadGuard` | Dispatch-time reservation contract |
| `DiscoveryBalance` | Transactional snapshot-to-endpoint reconciler |
| `EndpointFactory`, `InFlightFactory` | Service-generation construction |
| `StreamingDiscoveryBalance` | Discovery-driven Tower service |
| `StreamingConfig`, `StreamEndPolicy` | Fairness and terminal behavior |

Enable the `discovery` feature for reconciliation APIs.

## `poise-tokio`

| API | Role |
| --- | --- |
| `TokioProbe` | Async probe operation |
| `ActiveHealthRunner` | Timer and timeout adapter |
| `ProbeRunnerConfig`, `ProbeTimeoutPolicy` | Runtime-specific probe behavior |
| `next_snapshot`, `wait_for_revision` | Allocation-free async discovery waits |

Features `health` and `discovery` are independently selectable.

## `poise-observe`

| API | Role |
| --- | --- |
| `Observer`, `NoopObserver`, `Fanout` | Portable event sink composition |
| `ObservedPolicy` | Selection decorator |
| `Attempt` | RAII attempt lifecycle |
| `Metrics`, `MetricsSnapshot` | Fixed-cardinality cumulative counters |
| `DecisionEvent`, `AttemptEvent` | Structured portable events |
| `TracingObserver`, `TracedPolicy` | Optional tracing integration |
| `TowerObserver` | Optional readiness-failure adapter |

The `tracing` and `tower` features are off by default.

## Documentation layers

- This book explains contracts, composition, and operations.
- Crate-level rustdoc is the authoritative signature and feature reference.
- Source tests are executable examples of edge behavior.
- Focused policy chapters define arithmetic and churn guarantees.
- The live showcase reports verification provenance, not API documentation.
