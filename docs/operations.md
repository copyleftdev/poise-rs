# Operating Poise

Operating a load balancer means observing the control loop without turning
high-cardinality request data into a second failure mode.

## Production-readiness checklist

### Membership

- Stable backend identities survive ordinary discovery refreshes.
- Revisions increase monotonically and are logged at reconciliation boundaries.
- Duplicate identities fail the update rather than overwrite silently.
- Draining has a defined completion condition.
- Stream termination chooses last-known-good or fail-closed deliberately.

### Selection

- Stochastic seeds can be recorded in simulations and incident reproductions.
- Key encoding is canonical and documented.
- Weight changes are reviewed as traffic changes, not harmless metadata edits.
- Custom policies are tested for in-bounds eligible selection.
- Affinity retry behavior is explicit.

### Dispatch

- Every call follows a successful Tower readiness poll.
- Endpoint generation changes replace readiness and load state together.
- Capacity reservation failure has bounded reselection behavior.
- Cancellation is preserved when a response future is abandoned.
- Retry and hedge budgets live outside the selection policy.

### Health

- Active probe timing and timeout classification are documented.
- Passive circuit thresholds are calibrated to request rate.
- Half-open limits cannot exceed safe recovery traffic.
- Outlier ejection preserves minimum healthy capacity.
- Operator draining always outranks automated recovery.

### Observation

- Metric labels remain bounded.
- Stable identity appears in sampled logs or traces, not built-in metric labels.
- Healthy, spillover, and panic topology modes are distinguishable.
- Readiness failures have both a bounded counter and rate-limited diagnostics.
- Saturated counters are detectable during export.

## Recommended signals

The built-in `Metrics` recorder exposes a deliberately fixed surface:

- decision results by `DecisionKind`;
- attempts by `AttemptKind`;
- readiness-failure count;
- cumulative attempt-latency buckets;
- attempt-latency sum.

Derive rates and ratios in the monitoring system. Avoid resetting shared
counters on scrape; snapshots are cumulative.

Application-level telemetry may add bounded dimensions such as service name,
cluster, or deployment environment. Never add raw request keys, endpoint error
strings, or unbounded backend identities as metric labels.

## Suggested service-level indicators

| Indicator | Numerator | Denominator |
| --- | --- | --- |
| Selection availability | selected decisions | all decisions |
| Eligible-pool exhaustion | no-eligible decisions | all decisions |
| Dispatch completion | success + failure + overload | all attempts |
| Cancellation ratio | cancellations | all attempts |
| Readiness isolation | readiness failures | completed attempts |
| Panic-routing exposure | panic decisions | topology decisions |

Interpretation depends on traffic. A high cancellation rate may reflect client
deadlines rather than backend failure.

## Incident runbook

### No eligible candidates

1. Confirm whether membership is empty or non-empty.
2. Record current discovery revision and publisher state.
3. Separate draining, unavailable, circuit-open, and outlier-ejected members.
4. Inspect whether panic routing is disabled or intentionally fail-closed.
5. Avoid force-enabling draining members merely to restore capacity.

### Uneven traffic

1. Confirm the policy family and candidate order.
2. Compare configured weights and actual eligible duration.
3. For stochastic policies, inspect a meaningful sample window.
4. For affinity, measure key distribution before endpoint distribution.
5. For load-aware policies, inspect metric generation ownership and stranded
   guards.

### Churn causes excessive remapping

1. Verify identity stability and canonical key encoding.
2. Distinguish reorder from addition, removal, and weight change.
3. Confirm ring or Maglev cache rebuild reason.
4. Compare the measured movement to that policy’s documented guarantee.
5. Check whether retry exclusions or application fallbacks add extra movement.

### Load never returns to zero

1. Find outstanding response futures.
2. Confirm cancellation paths drop their guards.
3. Check for deliberate leaks through forgotten guards in application code.
4. Verify endpoint generations are not sharing one tracker accidentally.
5. Reproduce with the Loom in-flight models before changing atomic ordering.

## Rollout strategy

For a new policy:

1. replay production-shaped keys or load in simulation;
2. shadow decisions without dispatching;
3. compare eligibility and selected identity to the current balancer;
4. canary one bounded traffic slice;
5. watch exhaustion, cancellation, readiness, and topology mode;
6. retain an immediate configuration rollback.

Do not interpret a distribution benchmark as a rollout plan. Failure modes
usually appear in membership, readiness, cancellation, or key skew.

## Capacity planning

Configured weight is a relative routing input. Capacity planning must also
account for:

- per-endpoint concurrency limits;
- request cost variance;
- retry amplification;
- health headroom;
- priority overprovisioning;
- locality failure;
- connection-pool and runtime limits.

The balancer can preserve a capacity model only if the supplied weights and load
signals describe the real service generation.
