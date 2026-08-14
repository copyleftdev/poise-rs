# Performance model

Poise documents algorithmic cost and allocation behavior, and does not publish
unsupported throughput claims. Criterion baselines now exist for `poise-core`
selection and membership-change cost, and `scripts/bench.sh` reproduces them.
Dispatch, health, and discovery remain unmeasured.

Read the recorded numbers for shape rather than magnitude. The constants belong
to one machine and one compiler; how a policy responds to candidate count and
to churn is the part that transfers to a different deployment.

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
| Ring hash | O(n) validation, then O(log p) lookup | O(p) points | none after rebuild |
| Maglev | O(n) validation, then O(1) lookup | O(m) table | none after rebuild |
| Priority weighted random | O(n) grouping/scans | reusable groups | none after warm-up |
| Locality weighted random | O(n) grouping/scans | reusable groups | none after warm-up |

`n` is candidate count, `p` ring-point count, and `m` Maglev table size.
Eligibility density and membership churn influence constants materially.

The two cached rows are stated carefully because their headline is easy to
misread, and this table previously did. A Maglev table lookup is `O(1)` and a
ring lookup is `O(log p)`, but neither is what a call to `pick` costs: every
safe pick first validates the cached table against the candidate slice, which
is `O(n)`. Measured cost therefore grows with candidate count for both, and the
constant-time lookup is not the term that dominates.

This does not make them slow. Validating a candidate is far cheaper than
hashing one, so at 512 members Maglev picks in about a fifth of the time plain
rendezvous takes, and about a fifteenth of bounded-load rendezvous. The
correction is to the asymptote, not to the ranking: a cached policy is cheap
because its per-candidate work is small, not because it avoids per-candidate
work.

## Recorded baselines

Median `pick` time from `benches/selection.rs`, 100 samples per point, on an
AMD Ryzen Threadripper PRO 5975WX with rustc 1.97.1. Reproduce with
`scripts/bench.sh`.

| Policy | 8 | 64 | 512 |
| --- | ---: | ---: | ---: |
| Round robin | 5.14 ns | 5.16 ns | 5.17 ns |
| Weighted random | 15.6 ns | 60.3 ns | 379 ns |
| Least loaded | 16.6 ns | 96.0 ns | 730 ns |
| Random | 19.8 ns | 148 ns | 1.17 µs |
| Maglev | 33.5 ns | 170 ns | 1.43 µs |
| Power of two choices | 37.8 ns | 209 ns | 1.35 µs |
| Ring hash | 56.8 ns | 237 ns | 1.59 µs |
| Priority weighted random | 75.8 ns | 861 ns | 5.35 µs |
| Locality weighted random | 112 ns | 971 ns | 6.97 µs |
| Rendezvous | 127 ns | 986 ns | 8.13 µs |
| Smooth weighted round robin | 237 ns | 1.71 µs | 14.9 µs |
| Weighted rendezvous | 352 ns | 2.39 µs | 18.2 µs |
| Bounded-load rendezvous | 413 ns | 2.82 µs | 21.2 µs |

Every candidate is eligible in these runs, which is the cheapest case for the
policies that scan until they find an eligible member and the most expensive
for those that must consider everything. Round robin is the one whose measured
shape differs from its worst case for that reason: it is flat here because the
next candidate is always eligible, and its `O(n)` row describes a slice where
that is not true.

Everything else tracks candidate count, and the affinity policies separate by
constant rather than by shape: rendezvous hashes every candidate, while ring
hash and Maglev only validate every candidate and then consult a table. That
constant is the whole practical difference between them at 512 members.

## Membership-change cost

Cached affinity policies compute a membership fingerprint from the fields that
affect their structure. An unchanged membership reuses the table. Relevant
identity, eligibility, order, or weight changes rebuild transactionally.

Rebuild failure preserves the previous live cache. Callers should expose rebuild
errors and avoid retrying them for every request without backoff or
configuration repair.

Measured cost of a pick that triggers a rebuild, from `benches/membership.rs`,
which flips one member in and out on every iteration:

| Policy | 8 | 64 | 512 |
| --- | ---: | ---: | ---: |
| Ring hash | 62.7 µs | 713 µs | 6.25 ms |
| Maglev | 1.20 ms | 1.48 ms | 1.65 ms |
| Smooth weighted round robin | 229 ns | 1.82 µs | 14.9 µs |

Three things follow, and none of them are visible from the complexity column
alone.

**Ring hash and Maglev trade places.** Ring rebuild is `O(p)` in ring points,
which scale with members, so it grows with the fleet. Maglev rebuild is `O(m)`
in a table size fixed by configuration, so it is nearly flat and its cost at
eight members is almost its cost at five hundred. Ring hash is an order of
magnitude cheaper on small fleets and several times more expensive on large
ones; on this machine they cross somewhere between 64 and 512 members. A
deployment choosing between them on churn cost should know which side of that
crossing it sits on.

**Rebuild dwarfs selection.** A 512-member ring rebuild costs roughly three
thousand times a steady-state pick at the same size. Rebuilds are amortized
across the requests that follow a membership change, so this is not a
per-request cost — but it is the cost of a rollout, and a deployment whose
membership changes faster than it can amortize a rebuild has chosen the wrong
policy, whatever its lookup complexity says.

**Smooth weighted round robin has no rebuild penalty at all.** Its churn numbers
match its steady-state numbers, because identity-keyed state migrates
incrementally rather than being rebuilt. That is the property the table calls
identity-keyed state migration, and it is the reason it appears here despite
caching nothing to rebuild.

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

`scripts/bench.sh` runs the in-tree benchmarks and accepts criterion's baseline
arguments, so `--save-baseline` before a change and `--baseline` after it
reports the regression directly. Unlike the mutation and model-checking
wrappers, it applies no CPU quota, memory ceiling, or `nice` level: those bound
a job whose cost is the problem, whereas here the measurement is the product,
and throttling it would not produce a conservative number but a wrong one. A
benchmark is bounded by running it on an idle machine. The script warns when
the load average suggests otherwise and leaves the decision to the operator.

## Interpreting results

A faster isolated pick is not necessarily a better production policy. A policy
that reduces backend queueing or preserves cache affinity can save far more time
than it spends selecting. Conversely, an O(1) table lookup can be a poor choice
when membership changes faster than rebuild cost can be amortized.

Benchmark the control objective, not only the function call.
