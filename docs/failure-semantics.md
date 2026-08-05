# Failure semantics

Load balancers fail in more ways than “no backend.” Poise keeps failure classes
separate so callers can choose retry, failover, alerting, and telemetry without
parsing strings.

## Selection failures

| Failure | Meaning | Typical response |
| --- | --- | --- |
| `Empty` | The candidate slice has no members | Check discovery readiness or bootstrap state |
| `NoEligibleCandidates` | Members exist, but all are excluded | Inspect health, draining, and operator state |
| `WeightOverflow` | Eligible weights cannot be summed safely | Reject the configuration |
| Invalid custom index | A custom policy violated the index contract | Treat as a policy defect |
| Policy-specific capacity/configuration error | A bound or cached structure cannot be constructed | Preserve previous valid policy state |

Empty and ineligible are deliberately not interchangeable. During startup an
empty directory may be expected; during an incident a fully ineligible
directory often means health automation or operator state excluded the fleet.

## Discovery failures

Discovery state is revisioned and transactional:

- stale or duplicate revisions are rejected before endpoint construction;
- duplicate live identities are rejected;
- revision overflow does not partially apply a batch;
- a factory failure leaves the previous live pool intact;
- dropping the publisher wakes subscribers and ends the stream after the final
  snapshot.

Applications choose what stream termination means. `StreamingDiscoveryBalance`
supports last-known-good and fail-closed modes because neither is universally
correct.

## Readiness failures

A Tower readiness error belongs to one endpoint. `Balance` quarantines that
endpoint. If another endpoint is ready, aggregate readiness can still succeed.
If the failure exhausts the usable pool, the error includes the endpoint index
at the time of failure.

Indices are diagnostic and ephemeral. Membership may change afterward; durable
logs should also capture stable candidate identity at the application boundary.

Calling without a retained readiness reservation is a caller error and returns
a selection failure instead of calling an unready service.

## Completion, failure, and cancellation

Poise distinguishes lifecycle from application outcome:

| Event | Load guard | Passive outcome |
| --- | --- | --- |
| Response success | complete | success |
| Service future returns error | complete | application classifies failure/overload |
| Pending future is dropped | cancel | cancellation |
| Panic unwinds through guard | cancel by drop | cancellation unless caller records otherwise |

A returned error is still completed work for latency and concurrency tracking.
Cancellation does not imply backend failure and is ignored by rolling outcome
windows.

## Circuit permit races

Passive circuits issue permits tied to an epoch. A late result from a prior
epoch cannot close, reopen, or increment the current circuit. Half-open probe
limits are reserved atomically.

Dropping a permit without completion is cancellation. It must release its
reservation without inventing success or failure.

## Active probe races

Only one due active probe can be reserved for a health state. Explicit healthy
or unhealthy results advance consecutive thresholds. Cancellation reschedules
without changing classification.

Forced operator status invalidates an outstanding probe generation. A late
probe result cannot overwrite the forced state.

## Arithmetic and saturation

Poise rejects arithmetic that would make a decision ambiguous:

- weight accumulation uses checked arithmetic;
- revision overflow aborts transactionally;
- bounded-load capacity overflow is an explicit error;
- fixed-cardinality observation counters saturate at `u64::MAX` rather than
  wrapping.

Saturation in telemetry is observable loss of further count precision, not a
signal that selection state wrapped.

## Panic routing

Topology panic is explicit policy behavior, not a global “ignore health” switch.
Depending on `PanicMode`, it may broaden eligibility to unhealthy candidates
inside the selected scope. Draining and operator opt-out remain excluded.

Record whether a decision used healthy, spillover, or panic mode. Without this
metadata, incident traces cannot distinguish ordinary routing from emergency
scope expansion.

## Error-handling rules

1. Match typed variants; do not parse `Display` output.
2. Keep the last coherent snapshot when a transactional update fails.
3. Do not retry configuration and invariant violations as transient network
   errors.
4. Preserve cancellation as its own outcome.
5. Rate-limit logs for per-endpoint readiness failures; use bounded counters for
   aggregate health.
6. Alert on persistent “no eligible candidates,” not on one expected startup
   empty result.
