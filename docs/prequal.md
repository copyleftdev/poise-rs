# Prequal proposal

**Status: partially implemented.** The probe pool described under
[Probe pool contract](#probe-pool-contract) exists in `poise-core` as
[`ProbePool`](api-map.md). No `Prequal` policy type exists: the hot-cold
lexicographic rule, the decision type, and the fallback behavior remain
proposals. Nothing in this chapter has appeared in a released crate.

Prequal is proposed as the next selection family: a policy that chooses among
replicas using asynchronously collected probes rather than candidate-attached
load metrics. It would close two [roadmap](roadmap.md) items, adaptive
concurrency and capacity-aware routing, and it is the natural home for the
retry and hedge exclusion also listed there.

## Why probing changes the contract

Every existing Poise policy reads a signal the caller attached to a candidate.
`LeastLoaded` and `PowerOfTwoChoices` compare `LoadMetric` samples;
`BoundedLoadRendezvous` compares sampled concurrency against a computed
capacity. All of them balance *load*.

The Prequal result is that load is the wrong quantity to equalize. Equalizing
requests-in-flight across replicas of differing speed drives the slow replicas
into their queueing regime while the fast ones idle. Latency is the quantity a
caller experiences, and it should be minimized subject to a load cap rather than
the other way around.

That inverts the signal path. The policy no longer reads a number hanging off a
candidate; it reads a pool of recent observations gathered out of band, and each
observation names a replica the policy may or may not still consider eligible.
This is a new boundary, and it is the reason this proposal exists as a document
before it exists as a type.

## The hot-cold lexicographic rule

Given a pool of probes, each carrying a replica identity, a requests-in-flight
count, and a latency estimate:

```text
threshold = rif_quantile(pool)
cold      = { probe in pool : probe.rif <= threshold }

selection = if cold is non-empty { argmin latency over cold }
            else                 { argmin rif     over pool }
```

The cold branch optimizes latency among replicas with spare capacity. The hot
branch degrades to load balancing precisely when no replica has spare capacity,
which is the only regime where equalizing load is the correct objective.

The threshold must be computed by exact rank selection over the bounded pool,
not by a floating-point quantile estimator. Poise policies are required to be
reproducible and mutation-testable, and an estimator that drifts with
accumulation order is neither. Latency comparison would use a total order in the
style of `LoadScore`, which already resolves `f64` comparison through
`total_cmp`.

## Probe pool contract

The pool is the novel component and carries the obligations that make the rule
safe. It is implemented as `ProbePool`, and the obligations below are its
documented contract rather than an open design question:

- **Bounded.** The pool holds at most a configured number of entries. Insertion
  past that bound evicts, and no code path grows it, matching the existing
  prohibition on unbounded internal collections.
- **Consumed on use.** A probe informs at most a configured number of decisions
  and is then removed. This is not an optimization. A probe that reports an idle
  replica and is readable by every concurrent selector produces a stampede onto
  that replica, which is the classic failure of stale-information balancing.
  Bounded reuse is what makes the pool safe to share.
- **Aged out.** An entry older than a configured maximum is not eligible to
  inform a decision regardless of its use count.
- **Not authoritative over eligibility.** A probe naming a candidate that is
  draining, unavailable, or absent from the current membership generation is
  discarded at decision time. Health and discovery keep precedence; probing is a
  ranking signal layered on top of them, exactly as load is today.

Bounded reuse only holds if reading an observation and charging its budget are
one indivisible step. `ProbePool::decide_at` therefore takes the ranking
function rather than returning the slice: expiry, selection, and charging happen
under a single lock, so two concurrent selectors cannot both spend the last use
of one observation. That also fixes where eligibility filtering belongs — inside
the caller's ranking function, which is the only place that can see both the
observations and the current candidate set.

## Determinism

Prequal would be the first policy whose inputs vary with wall-clock time, which
puts pressure on the law that a seeded policy replays exactly.

The law is preserved by keeping the sampling discipline the existing policies
already follow: `decide` takes one coherent view of the pool and reads it once,
in the same way `BoundedLoadRendezvous` samples every eligible `LoadMetric`
exactly once per decision. Given an identical pool state and an identical
candidate slice, the selection is identical. Time enters through pool
maintenance and never through the selection rule; every time-dependent pool
operation has an `_at` form that takes the reading instant, so tests and
simulations drive a synthetic clock rather than the wall clock.

## Crate placement

The split follows the existing tracker and policy separation, where `PeakEwma`
lives in `load.rs` and the policies that read it live in `policy/`:

| Component | Location | Runtime dependency |
| --- | --- | --- |
| `ProbePool` shared state | `poise-core`, beside `load.rs` | None |
| Hot-cold selection rule | `poise-core/src/policy/` | None |
| Probe issuance and scheduling | `poise-health` active probes | None |
| Optional probe timing driver | `poise-tokio` | Tokio |
| Regime counters | `poise-observe` | Optional |

The single rule this must not break: the paper couples probe rate to request
rate, which requires a scheduler, and `poise-core` has no runtime. The core may
only *observe* that probing is due; it may never drive it. Any design that
places probe scheduling in the core forfeits the runtime neutrality that
`poise-core`, `poise-discovery`, and `poise-health` currently guarantee, and
should be rejected on that basis alone.

## Decisions and errors

`decide` would return a decision exposing why the selection happened, following
`BoundedLoadDecision` and `PriorityDecision`:

- `selection`: the chosen candidate;
- `regime`: whether the cold branch, the hot branch, or a fallback produced it;
- the number of pool entries that informed the decision;
- the selected entry's requests-in-flight and latency, and the computed
  threshold.

A cold start is the interesting case. An empty pool is not an error, and
inventing a `PickError` variant for it would misreport a healthy system that has
simply not probed yet. The proposal is instead a fallback policy parameter,
defaulting to `PowerOfTwoChoices`, with the fallback reported through `regime`
so an operator can observe how often selection ran without probe data. Silent
fallback would violate the standing requirement that outcomes stay
distinguishable.

Empty and wholly ineligible slices keep the standard `PickError` distinction.
Scratch growth returns `StateCapacityExceeded`, as elsewhere.

## Divergence from the paper

Two deliberate departures, both of which must be documented as contract rather
than left implicit:

1. **Exact rank selection instead of an estimated quantile.** Required for
   reproducible replay and for mutation testing to be meaningful.
2. **No global assignment state.** As with
   [bounded-load affinity](bounded-load-affinity.md), Poise observes a borrowed
   snapshot and chooses one destination. It does not own the fleet-wide
   assignment, so it cannot claim the paper's system-level results.

The paper's constants are a starting point and not a contract. Pool size, reuse
limit, age limit, probe rate, and quantile must each be chosen deliberately,
documented with their operational meaning, and justified against measurements
rather than inherited.

## Verification plan

Mapped onto the evidence table in [testing](testing.md):

| Obligation | Evidence | State |
| --- | --- | --- |
| Pool retention edges | Unit tests and a compiling rustdoc example | Delivered |
| Shared pool | Loom models over concurrent consumption | Delivered |
| Pool interleavings | The `probe_pool` fuzz target | Delivered |
| Survivors | Contract tests holding `poise-core` at zero viable survivors | Delivered for the pool |
| Rule edge behavior | Unit tests and a compiling rustdoc example | Pending the policy |
| General laws | Proptest laws with committed regression seeds | Pending the policy |
| Seeded probe targeting | Exact replay plus distribution bounds | Pending the policy |
| Hot path | Criterion baselines, still an open roadmap gap | Pending |

The laws worth stating explicitly:

- selection is eligible and in bounds, or the precise error;
- if any probed replica is cold, the selection is cold and no cold replica has
  strictly lower latency;
- if every probed replica is hot, the selection has minimum
  requests-in-flight;
- raising the quantile never shrinks the cold set;
- the pool never exceeds its bound, and no entry outlives its reuse or age
  limit;
- under concurrent selection against one idle replica, no more probes are spent
  on it than the reuse limit permits, which is the property that separates this
  from naive stale-information balancing;
- a deliberately slow reference implementation of the rule, kept in the test
  support module, agrees with the optimized path on generated input.

The last three are pool properties and now hold. The `probe_pool` fuzz target
drives arbitrary record, expire, and select interleavings under the limits in
[fuzzing](fuzzing.md), and the Loom models cover the concurrent reuse bound.
The remaining laws describe the selection rule and land with the policy.

## Open questions

- Which quantile, and does it need to adapt with observed load, or is a fixed
  configured rank defensible for a library that does not own the fleet?
- Does the pool key on candidate identity or on the discovery generation plus
  index, and what happens to entries that survive a membership change?
- Should probe issuance reuse the `poise-health` active prober directly, or does
  a latency probe differ enough from a health probe to warrant its own type?
- Does the fallback belong as a type parameter, or should a caller compose it
  explicitly and keep `Prequal` total?

## References

- [Load is not what you should balance: Introducing Prequal](https://www.usenix.org/conference/nsdi24/presentation/wydrowski),
  Wydrowski, Kleinberg, Rumble, and Archer, USENIX NSDI 2024.
- [Architecture](architecture.md) for the boundary this policy must respect.
- [Load and feedback](load-and-feedback.md) for the tracker contract it extends.
