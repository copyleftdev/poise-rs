# Load and feedback

Poise separates a load measurement from an admission reservation and from an
attempt outcome. Conflating those concepts is a common source of stranded
capacity and unstable feedback loops.

## The `LoadMetric` boundary

A load-aware candidate exposes a metric whose smaller values represent less
load. Built-in integer metrics, `InFlight`, and `PeakEwma` implement the
contract.

A policy samples a metric during one decision. The value can change immediately
afterward, so measurement alone is not a hard limit.

## In-flight accounting

`InFlight` is a shared atomic counter. Reserving returns an RAII guard:

```text
counter N ── reserve ──▶ N + 1
              │
              ├─ complete ──▶ N
              └─ drop/cancel ▶ N
```

The counter is balanced under normal completion, early return, future
cancellation, and panic unwinding. `try_acquire` adds an atomic limit check;
rejection does not increment the counter.

Selection and admission can still race:

1. two selectors observe the same available capacity;
2. both choose the endpoint;
3. only one atomic reservation succeeds.

The loser must select again or return overload according to the application’s
budget. Never assume a bounded-load policy replaces atomic admission.

## Peak EWMA

`PeakEwma` combines decaying observed latency with current concurrency. A new
high latency raises the score immediately; without new observations, the
latency component decays toward its configured floor.

The tracker uses a caller-supplied monotonic clock at its core boundary. This
keeps simulated time, test time, and runtime time coherent.

Configuration rejects zero decay or default-latency parameters. Completion
records latency and releases concurrency; cancellation releases concurrency
without recording a latency sample.

## Outcome classification

The portable `Outcome` classes are:

- success;
- failure;
- overload;
- cancellation.

Overload can carry a larger penalty than ordinary failure in an
`OutcomeWindow`. Cancellation is not a backend observation and therefore does
not enter success-rate statistics.

## Rolling windows

`OutcomeWindow` stores bounded recent history. Its minimum-sample gate avoids
penalizing a cold endpoint based on one early failure. When capacity is reached,
the oldest observation is evicted.

The window exposes a penalty metric suitable for composition with load-aware
policies. It does not automatically open a circuit or eject a host; the control
plane chooses how to apply the signal.

## Group-relative outliers

`OutlierDetector` compares sufficiently sampled hosts to their group baseline.
It returns deterministic worst-first candidate indices below a configured
standard-deviation threshold.

Two caps protect availability:

- maximum ejection percentage;
- minimum healthy group size.

Detection is pure. Scheduling ejection duration, changing health state, and
deciding when to re-admit remain control-plane responsibilities.

## Feedback-loop discipline

Any feedback controller can oscillate. Establish:

- a measurement window longer than individual request jitter;
- a minimum sample count;
- explicit overload weighting;
- bounded ejection;
- cooldown before half-open probes;
- monotonic time from one domain;
- dashboards that distinguish selection, admission, and outcome.

Avoid feeding exporter-delayed metrics back into the hot path. Use the shared
in-process tracker that belongs to the service generation.

## Generation safety

Load state belongs to the concrete endpoint generation, not merely its logical
key. When discovery reuses a key for a new backend allocation, build a new
tracker. Outstanding futures from the old generation retain and release the old
guard without modifying the replacement.
