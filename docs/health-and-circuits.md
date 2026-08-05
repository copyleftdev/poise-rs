# Health and circuits

`poise-health` provides executor-neutral health state. It does not send
network probes, sleep, spawn tasks, or decide which application result is a
failure. Those choices remain with adapters and protocol code.

## Compose independent signals

`HealthChecked<Backend, Health>` wraps an existing candidate and adds a
`HealthSignal`. Candidate identity, configured weight, topology metadata, and
load delegate to the inner backend. Eligibility requires both the inner
candidate and health signal to permit selection.

This composition preserves administrative intent:

- a draining backend does not become selectable because its probe is healthy;
- an unavailable backend does not become ready because a circuit closes;
- nested health wrappers can express multiple independent gates.

## Passive circuit states

```text
Closed ── failure threshold ──▶ Open
  ▲                              │
  │                              │ cooldown elapsed
  │                              ▼
  └── success threshold ───── Half-open
                                 │
                                 └── probe failure ──▶ Open
```

`CircuitConfig` controls:

- consecutive failures required to open;
- open cooldown duration;
- successful half-open probes required to close;
- maximum concurrent half-open permits.

Configuration rejects a zero open duration. Large durations remain valid
without requiring eager future-instant arithmetic.

## Permit semantics

`PassiveHealth::try_acquire` returns a `CircuitPermit` or a typed rejection.
A permit reserves half-open capacity when applicable. Exactly one terminal
action should follow:

- complete with a classified outcome;
- cancel explicitly;
- drop, which cancels.

Every permit carries the circuit epoch from which it was issued. Late completion
from an old epoch is ignored, preventing reordered responses from corrupting a
newer forced or reopened state.

## Active health

`ActiveHealth` owns classification and scheduling state, while the caller owns
I/O and time. Its configuration defines:

- healthy-result threshold;
- unhealthy-result threshold;
- probe interval;
- initial unknown policy.

Only one caller can reserve a due probe. Completion with `Healthy` or
`Unhealthy` advances consecutive thresholds. Cancellation preserves the
current classification and schedules another attempt.

Use the clock-aware methods when the adapter supplies a simulated or
runtime-specific `Instant`. Do not reserve using one time domain and complete
using another.

## Tokio adapter

`poise-tokio::ActiveHealthRunner` supplies timing around a caller-defined
`TokioProbe`. Timeout policy is explicit:

- classify timeout as unhealthy; or
- cancel without changing classification.

Dropping the runner future finalizes its reservation as cancellation. Repeated
probe scheduling follows Tokio’s clock, including paused test time.

## Outcome windows versus circuits

These mechanisms answer different questions:

| Mechanism | Question |
| --- | --- |
| Consecutive-failure circuit | Has this endpoint failed repeatedly right now? |
| Rolling outcome window | What fraction and weighted kind of recent attempts failed? |
| Outlier detector | Is this endpoint materially worse than sufficiently sampled peers? |
| Active health | Does an independent probe currently classify it healthy? |

Combining them is reasonable, but define precedence. A typical order is
administrative state → active health → circuit permission → outlier ejection.

## Choosing thresholds

Thresholds are workload parameters, not library defaults to copy blindly.
Derive them from:

- ordinary request rate per endpoint;
- expected transient failure bursts;
- probe interval and timeout;
- acceptable time to detect and time to recover;
- minimum healthy capacity;
- retry amplification.

A failure threshold of three means something very different at one request per
minute and ten thousand requests per second.

## Operational invariants

- Force operations take effect immediately and invalidate stale probes.
- Cancellation never counts as backend failure.
- Half-open concurrency is an atomic reservation, not a statistical target.
- Group-relative ejection never exceeds both configured availability caps.
- Health wrappers never revive administrative draining or opt-out.
- Control-plane actions should record the signal and epoch that caused them.
