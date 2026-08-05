# Performance model

Poise documents algorithmic cost and allocation behavior. It does not publish
unsupported throughput claims: criterion baselines across representative
candidate counts remain a roadmap item.

## Selection cost

| Policy | Pick time | Persistent state | Pick allocation |
| --- | ---: | ---: | ---: |
| Round robin | O(n) worst case | one cursor | none |
| Smooth weighted round robin | O(n) | identity-keyed weights | none after state growth |
| Random | O(n) | RNG | none |
| Weighted random | O(n), two scans | RNG | none |
| Least loaded | O(n) | tie cursor | none |
| Power of two choices | O(n) reservoir scan | RNG | none |
| Rendezvous | O(n) hashes | hasher | none |
| Weighted rendezvous | O(n) hashes/transforms | hasher | none |
| Bounded-load rendezvous | O(n log n) ranking | reusable scratch | none after warm-up |
| Ring hash | O(log p) lookup | O(p) points | none after rebuild |
| Maglev | O(1) lookup | O(m) table | none after rebuild |
| Priority weighted random | O(n) grouping/scans | reusable groups | none after warm-up |
| Locality weighted random | O(n) grouping/scans | reusable groups | none after warm-up |

`n` is candidate count, `p` ring-point count, and `m` Maglev table size.
Eligibility density and membership churn influence constants materially.

## Membership-change cost

Cached affinity policies compute a membership fingerprint from the fields that
affect their structure. An unchanged membership reuses the table. Relevant
identity, eligibility, order, or weight changes rebuild transactionally.

Rebuild failure preserves the previous live cache. Callers should expose rebuild
errors and avoid retrying them for every request without backoff or
configuration repair.

## Dispatch cost

`poise-tower::Balance::poll_ready` scans endpoint readiness in O(n) and performs
no allocation. A ready service retains its reservation until selected.

`call` performs the policy’s normal selection cost, acquires the endpoint load
guard, and returns a concrete response future without boxing it.

Snapshot reconciliation is control-plane work. Unchanged service generations
retain service, readiness, and load state; new generations are built before
commit.

## Atomic costs

`InFlight` uses atomic updates for reservation and release. `PeakEwma` couples
atomic concurrency with synchronized estimator state. `Metrics` uses relaxed
atomic saturating counters.

Contention depends on how policy and tracker instances are shared. One global
policy mutex may dominate every algorithmic difference in the table above.

## Memory bounds

- Discovery snapshots own one immutable membership vector per live revision
  retained by readers.
- Outcome windows have configured fixed capacity.
- Metrics own 28 counters regardless of traffic diversity.
- Ring and Maglev caches are bounded by validated configuration.
- The showcase renderer is unrelated to library runtime and caps particles and
  device pixel ratio in the browser.

## How to benchmark your deployment

Measure at least:

1. candidate counts at p50, p95, and maximum;
2. eligibility ratios during healthy and degraded operation;
3. membership update frequency;
4. affinity key skew;
5. tracker contention at real worker counts;
6. policy construction and cache rebuild separately from steady-state pick;
7. complete readiness-to-response lifetime, not selection alone.

Use fixed seeds for comparable stochastic runs. Prevent the optimizer from
removing decisions. Report allocator activity and tail latency, not only mean
throughput.

## Interpreting results

A faster isolated pick is not necessarily a better production policy. A policy
that reduces backend queueing or preserves cache affinity can save far more time
than it spends selecting. Conversely, an O(1) table lookup can be a poor choice
when membership changes faster than rebuild cost can be amortized.

Benchmark the control objective, not only the function call.
