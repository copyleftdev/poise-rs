# Compose the system

A production balancer is a pipeline of ownership boundaries. Poise works best
when each boundary has one job and communicates through explicit state.

```text
control plane                                      data plane

source → Directory → Snapshot ──┐
                                ├→ candidate view → Policy → Tower service
probe  → health state ──────────┤                         │
result → circuit/load/metrics ──┘←────────────────────────┘
```

## The five-stage path

### 1. Publish coherent membership

`poise-discovery::Directory` is a single-writer reconciliation boundary.
Apply a batch, obtain one new revision, then publish an immutable `Snapshot`.
Readers keep older snapshots alive safely while the new generation becomes
visible atomically.

Treat discovery as coalesced state, not an event log. A slow subscriber may move
directly from revision 10 to revision 14; revision 14 must contain the complete
coherent membership it needs.

### 2. Derive eligibility

Candidate eligibility should combine independent signals without losing their
origins:

- administrative state: ready, draining, unavailable;
- active probe classification;
- passive circuit permission;
- group-relative outlier decision;
- topology scope;
- atomic capacity admission.

`HealthChecked` composes health with an existing candidate. It does not mutate
the underlying administrative state. Draining remains draining even if health
is otherwise good.

### 3. Select without dispatching

A policy sees a coherent candidate slice and optional request context. It
returns an index and, for advanced families, structured decision metadata.

Selection must not consume a Tower readiness permit, increment a load tracker,
or start network work. Those effects belong to dispatch.

### 4. Retain readiness through dispatch

`poise-tower::Balance` polls endpoint readiness before selection. A ready
endpoint retains its service reservation. `call` lets the policy choose only
among currently ready candidates and consumes exactly the selected reservation.

This ordering avoids the classic Tower bug in which a balancer selects a
service, drops the readiness permit, then calls a service whose capacity has
already changed.

### 5. Feed back a classified outcome

The response future owns the selected endpoint’s load guard:

- a returned success completes it;
- a returned service error also completes it;
- dropping the pending future records cancellation.

Protocol code then maps the result into Poise’s portable `Outcome` classes for
passive health and observation.

## Reference deployment shape

```text
                         ┌───────────────┐
DNS / xDS / config ─────▶│ Directory     │
                         └──────┬────────┘
                                │ immutable revision
                         ┌──────▼────────┐
active probes ──────────▶│ candidate view│◀── passive circuit
                         └──────┬────────┘
                                │ eligible slice
                         ┌──────▼────────┐
request key ────────────▶│ Policy        │
                         └──────┬────────┘
                                │ Selection
                         ┌──────▼────────┐
                         │ Tower Balance │
                         └──────┬────────┘
                                │ response future + load guard
             metrics / tracing ◀┴▶ outcome window / circuit
```

The diagram is a topology, not a requirement that every stage be a separate
task. Small systems can keep discovery static and use only `poise-core` plus
`poise-tower`.

## Choose the ownership boundary

| State | Recommended owner | Reason |
| --- | --- | --- |
| Backend identity and configured weight | Discovery snapshot | Must remain coherent across a selection |
| Policy RNG, cursor, or hash table | Policy instance | Defines selection sequence and cached membership |
| Tower readiness | Endpoint service | A permit belongs to one service generation |
| In-flight count or EWMA | Endpoint generation | Results must update the service that handled them |
| Circuit epoch | Health wrapper | Late permits must not mutate a newer epoch |
| Metrics counters | Shared observer | Clones should aggregate without changing selection |
| Retry budget | Application or protocol layer | Retries change request semantics |

The most important identity boundary is the service generation. Reusing a
logical key with a newly allocated backend should build a new endpoint and load
tracker; otherwise late results from the old service pollute the new service.

## Snapshot reconciliation

With the `discovery` feature, `poise-tower` stages endpoint builds before
committing a new pool:

1. reject stale revisions and duplicate live identities;
2. retain endpoints whose key and backend allocation are unchanged;
3. build all new service generations;
4. abort without changing the live pool if any build fails;
5. atomically replace ordering and membership;
6. allow physically retired endpoints to remain owned by outstanding futures.

This is transactional application, not merely transactional publication.

## Context projection

Affinity policies need a stable key, but the policy should not own or clone the
request. Implement `RequestContext<Request>` to borrow the routing field.
`UseRequest` is available when the entire request value is already the key.

Canonicalize the key before it reaches a keyed policy. Equivalent user IDs with
different case, Unicode normalization, or serialization must not hash as
different identities unless that distinction is intentional.

## Retrying correctly

Poise selects one attempt. Retry orchestration remains outside the policy.
Before retrying, answer:

- whether the same affinity key should return to the same backend;
- whether the failed endpoint is excluded from the next attempt;
- whether the first attempt was cancelled or completed;
- whether capacity is released before the next selection;
- whether the retry consumes a shared budget.

Retrying blindly through the same deterministic affinity policy can select the
same failing owner repeatedly. Retry/hedge exclusion is therefore a roadmap
item rather than an implicit behavior.

## Minimal compositions

| Need | Composition |
| --- | --- |
| Static, synchronous choice | `poise-core` |
| Static Tower pool | `poise-core + poise-tower` |
| Versioned pool updates | add `poise-discovery`, enable Tower `discovery` |
| Passive health | wrap candidates with `poise-health` |
| Timed active probes | add `poise-tokio` |
| Bounded metrics | add `poise-observe` |
| Structured traces | enable `poise-observe/tracing` |

Start with the smallest composition. Add a layer when its state and failure
semantics are understood.
