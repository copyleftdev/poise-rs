# Tower dispatch contract

`poise-tower` turns a collection of candidate metadata and Tower services into
one policy-driven `Service`. It deliberately does not create a runtime, spawn a
task, buffer requests, or choose retry behavior.

## Endpoint model

An `Endpoint<C, S, L>` owns three durable values:

- candidate metadata `C` for identity, weight, and administrative or health
  eligibility;
- Tower service `S` for readiness and dispatch;
- load tracker `L`, exposed to load-aware Poise policies and held for the
  lifetime of each dispatched response future.

The default tracker is an unbounded `InFlight`. `Endpoint::with_tracker` accepts
another `LoadTracker`, including `PeakEwma`. The endpoint's policy load is this
dispatch tracker, not the load field of `C`; this ensures the metric describes
the service instance that will actually receive the call.

## Readiness lifecycle

```text
Idle --poll_ready(Ok)--> Ready --selected call--> Idle
  |                         |
  +--poll_ready(Err)--> Failed --explicit reset--+
```

`Balance::poll_ready` scans all administratively eligible idle endpoints. A
pending endpoint remains idle and is polled again when its service wakes the
caller. A ready endpoint retains its reservation and is not polled again until
selected. This matters for services whose `poll_ready` reserves a permit.

Readiness failure quarantines one endpoint. If another endpoint is ready, the
aggregate remains ready. Install `with_readiness_observer` to record every such
error. If the failure exhausts the usable pool, `poll_ready` returns it with the
endpoint index. Reset failed services explicitly after repairing or replacing
them.

As required by Tower, callers should await readiness before each call. Calling
without a retained reservation returns a selection error rather than invoking
an unready inner service.

## Completion and cancellation

Dispatch reserves the selected endpoint's tracker before calling its service.
The returned `ResponseFuture` owns both the service future and load guard:

- `Ok(response)` completes the load guard;
- `Err(error)` also completes it and preserves the dispatch-time endpoint
  index;
- dropping the pending future drops the guard as cancellation.

No response future is boxed by the adapter. `poll_ready` takes `O(n)` time over
the endpoint set and performs no allocation. `call` adds the selected policy's
normal time complexity.

## Affinity context

`Balance::new` supplies unit context for request-independent policies. Use
`with_context(UseRequest)` when the entire request is an affinity key. For
structured requests, implement `RequestContext<Request>` to borrow only the
stable routing field; the projection does not need to allocate or clone it.

## Membership changes

`push`, `remove`, and `endpoints_mut` support controlled membership changes.
New endpoints always enter idle. Mutable access to a service invalidates its
readiness reservation. An error's endpoint index describes the ordering at
dispatch or readiness time and may become stale after later membership edits;
use candidate identity for durable telemetry labels.

Stream-driven reconciliation with `poise-discovery` is a separate adapter so
the base Tower service remains runtime-neutral. Enable the optional `discovery`
feature for the versioned static reconciler described in
[Discovery and Tower](discovery-tower.md).
