# Prequal proposal

**Status: partially implemented.** The probe pool described under
[Probe pool contract](#probe-pool-contract) exists in `poise-core` as
[`ProbePool`](api-map.md), including the candidate-aware selection described
under [Selecting against a candidate set](#selecting-against-a-candidate-set). No `Prequal` policy type exists: the hot-cold
lexicographic rule, the decision type, and the fallback behavior remain
proposals. Nothing in this chapter has appeared in a released crate.

Prequal is proposed as the next selection family: a policy that chooses among
replicas using asynchronously collected probes rather than candidate-attached
load metrics. It would give the adaptive-concurrency and capacity-aware-routing
[roadmap](roadmap.md) entries the signal they need — the entries themselves
describe deployments, and a primitive supplies the piece rather than closing
them. What this chapter does *not* propose is set out under
[What this chapter is not proposing](#what-this-chapter-is-not-proposing), which
is worth reading first if the length here suggests otherwise.

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

The paper is emphatic that this lexicographic shape is doing the work, not the
two signals on their own: hot-cold beat RIF-only control, and it beat *every*
non-trivial linear combination of RIF and latency they tried. A weighted score
is therefore not a simpler equivalent of this rule, and should not be offered as
one. The ordering encodes a hierarchy rather than a trade-off — latency is worth
optimizing, but a replica staying inside its memory allocation is a constraint,
and constraints do not average with objectives.

The threshold must be computed by exact rank selection over the bounded pool,
not by a floating-point quantile estimator. Poise policies are required to be
reproducible and mutation-testable, and an estimator that drifts with
accumulation order is neither. Latency comparison would use a total order in the
style of `LoadScore`, which already resolves `f64` comparison through
`total_cmp`.

Note what the quantile is taken over. In the paper, a client maintains an
estimate of the RIF distribution *across replicas* from recent probe responses,
and `Q_RIF` is a fixed rank into that estimate. The adaptivity lives in the
distribution, not in the rank. For a bounded pool this collapses to an exact
rank over the pool's own entries, which is the same computation Poise already
requires for reproducibility — so the paper's design and this repository's
determinism constraint agree here rather than conflict.

## Parameters from the deployment

The paper's operating points, recorded so future choices are made against
evidence rather than invention. These are inputs to a decision, not a contract:

| Parameter | Paper value | Meaning |
| --- | ---: | --- |
| `Q_RIF` | `2^-0.25` ≈ 0.84 | Rank into the estimated RIF distribution above which a probe is hot. Good range `[0.6, 0.9]`; even `0` works, degenerating to RIF-only control |
| Pool size `m` | 16 | "A pool size of 16 suffices"; gains beyond it are modest |
| Probe age limit | 1s | With a 3ms probe RPC timeout in YouTube, 1ms elsewhere at Google |
| `r_probe` | 3 per query | May be fractional, even below one; behavior is insensitive to it until it drops under one probe per query |
| `r_remove` | 1 per query | Probes deleted per query to counter degradation, may be fractional |
| `δ` | 1 | Governs the net rate at which probes accumulate |
| `b_reuse` | *derived* | `max{1, (1 + δ) / ((1 - m/n)·r_probe - r_remove)}`, where `n` is the replica count |

The last row is the one that matters most for this repository. Prequal does not
configure a reuse budget; it *computes* one from the probing rate, the removal
rate, the pool size, and the replica count, and randomly rounds a fractional
result to preserve its expectation. `ProbePoolConfig::max_uses` is a raw
constant by comparison. That is the right shape for a storage primitive — the
pool knows none of those four quantities — but it means the number is only
meaningful when something upstream derives it, and that derivation belongs with
the policy rather than with a caller's guess.

That derivation is partial, and any implementation must say what it does outside
the domain. The expression is only defined where

```text
n > m    and    (1 - m/n) * r_probe > r_remove
```

The first condition is the ordinary case — fewer pool slots than replicas — and
the pool is pointless without it. The second is the real constraint: it says
probes must arrive faster than the pool sheds them. It is not a remote edge.
With the paper's own `m = 16`, `r_probe = 3`, `r_remove = 1`, a replica count of
24 makes the denominator exactly zero, and any smaller fleet makes it negative.
Reaching for `max{1, ...}` does not rescue this; a zero denominator is a
division error before the maximum is ever taken, and a negative one silently
returns a budget of 1 that is not the formula's answer but the clamp's.

The condition has an operational reading: at that probe and removal rate the
pool cannot sustain itself, so no reuse budget makes it self-supporting.
Reuse is not the lever there — `r_probe` must rise or `r_remove` must fall.
A policy computing this must therefore validate the domain first and report the
combination as a configuration error, in the manner of `ProbePoolConfigError`,
rather than dividing and hoping. Depletion is then a tuning outcome the operator
chose, visible through the fallback regime, and not an arithmetic accident.

## Probe pool contract

The pool is the novel component and carries the obligations that make the rule
safe. It is implemented as `ProbePool`, and the obligations below are its
documented contract rather than an open design question. They are not the whole
set: a further obligation, degradation control, is described after them and has
no implementation yet.

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

### Degradation, and the obligation still missing

The paper removes probes for three distinct reasons, and the pool as
implemented addresses only two of them. Staleness is covered by the age limit,
and depletion by bounded reuse. The third is **degradation**, and it is a
selection-induced bias rather than a timing problem: selection consumes the
probes reporting lightly loaded replicas first, so what accumulates in the pool
over time is disproportionately the probes reporting heavily loaded ones. A pool
that only ages and only consumes drifts toward describing the fleet as busier
than it is, and the rule reads that drift as fact.

Prequal's answer is to delete probes at a configured rate `r_remove` per query,
alternating between two rules: remove the oldest, and remove the *worst* by the
same ranking used for selection, run in reverse — the hot probe with the highest
RIF if any probe is hot, otherwise the cold probe with the highest latency.
Alternating is what makes one mechanism cover both staleness and degradation.

This is the one paper obligation with no counterpart in `ProbePool`, and it is
load-bearing rather than an optimization: without it the pool develops exactly
the bias the rule is least able to detect, because a uniformly pessimistic pool
still looks internally consistent.

Removing the worst probe requires ranking, and the pool deliberately has no
opinion about ranking. The resolution proposed here follows the shape
`decide_at` already established: the caller supplies the ordering, the pool owns
the mechanics and the lock. A removal entry point would take a ranking function,
would apply it in reverse, and would drop under the same lock that governs
selection and charging. No such method exists yet. The policy would keep the
rule; the pool would keep retention. `r_remove` belongs to the policy layer for
the same reason `b_reuse` does — it is denominated in queries, and the pool does
not know what a query is.

### Fallback threshold

An empty pool is not the only cold condition. The paper falls back to a
uniformly random replica when the pool is empty, and reports that it is useful
to invoke that fallback "whenever the pool occupancy drops below 2" — a pool
holding a single probe offers no choice, so ranking it is a formality that
launders one stale observation into a decision. `ProbeDecisionError::NoProbes`
currently reports only true emptiness. The occupancy threshold at which a
caller should prefer its fallback is a policy parameter, and the decision must
report which branch produced it either way.

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

## What a probe reports

The two fields are not the independent scalars their types suggest, and the
server side owes more than reading two counters.

RIF is a counter read. The latency estimate is conditioned on it: when a query
finishes, the paper's server module records that query's latency *tagged with
the RIF counter value at its arrival*; answering a probe then reports the median
of recent latencies at or near the current RIF. At moderate query rates those
samples come entirely from the last few milliseconds.

So a probe answers "how fast is this replica right now, at the concurrency it is
currently running at," not "how long did the probe take." `ProbeReading`
documents its latency as the observed service time for the probe itself, which
is a weaker and different claim — a reporter that measures probe round-trip time
satisfies the type while violating the contract the rule depends on. The pool
cannot enforce this, so it belongs in the documented obligation on whoever
produces readings.

## Divergence from the paper

Deliberate departures, documented as contract rather than left implicit:

1. **Exact rank selection instead of an estimated quantile.** Required for
   reproducible replay and for mutation testing to be meaningful. As noted
   above, over a bounded pool this coincides with the paper's construction
   rather than opposing it.
2. **No global assignment state.** As with
   [bounded-load affinity](bounded-load-affinity.md), Poise observes a borrowed
   snapshot and chooses one destination. It does not own the fleet-wide
   assignment, so it cannot claim the paper's system-level results.
3. **Reuse does not compensate.** When a Prequal client sends a query to a
   replica it holds a probe for, it increments the RIF recorded on that probe,
   since it has just added load the probe predates. `ProbePool` charges the
   reuse budget and returns the reading unchanged. Their own note is that they
   would like to age the latency estimate the same way and do not, so this is a
   known-partial mitigation in the paper too. Compensation needs the rule's
   units and belongs with the policy; until it exists, a reused probe overstates
   how idle its replica still is.
4. **Eviction compares observation instants.** A full pool drops the incoming
   observation when it is staler than everything retained, rather than always
   evicting the oldest resident. The paper simply drops the oldest, and does not
   discuss probes completing out of order. Ours is the stricter rule and costs
   nothing, but it is ours, not theirs.

The paper's constants are a starting point and not a contract. Pool size, reuse
limit, age limit, probe rate, and quantile must each be chosen deliberately,
documented with their operational meaning, and justified against measurements
rather than inherited — with the caveat that `b_reuse` is derived rather than
chosen, and choosing it directly is already a departure.

## Verification plan

Mapped onto the evidence table in [testing](testing.md):

| Obligation | Evidence | State |
| --- | --- | --- |
| Pool retention edges | Unit tests and a compiling rustdoc example | Delivered |
| Shared pool | Loom models over concurrent consumption | Delivered |
| Pool interleavings | The `probe_pool` fuzz target | Delivered |
| Survivors | The mutation campaign holding `poise-core` at zero viable survivors, with pool contract tests among what catches them | Delivered for the pool |
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

The bound-and-expiry law and the concurrent-reuse law are pool properties and
now hold. The `probe_pool` fuzz target drives arbitrary record, expire, and
select interleavings under the limits in [fuzzing](fuzzing.md), and the Loom
models cover the concurrent reuse bound. Reference-implementation parity is
listed with them, but it validates the selection rule rather than the pool, so
it stays pending with the rest of the rule laws and lands with the policy.

## Settled by the paper

Three questions this chapter opened with are answered by the source rather than
left to taste.

**The quantile is a fixed rank, and that is defensible.** `Q_RIF` is a
configured constant; what adapts is the estimated RIF distribution it indexes
into. A library that does not own the fleet can hold the rank fixed and let the
pool supply the distribution, which is what a bounded pool does by construction.
Default to the paper's range and treat `0` as a supported setting rather than a
degenerate one, since it selects RIF-only control deliberately.

**Probe issuance warrants its own type.** A probe carries a RIF count and an
RIF-conditioned latency estimate, runs against single-digit-millisecond
timeouts, and in synchronous mode carries request information the replica reads.
A health probe answers whether a replica should receive traffic at all. Sharing
scheduling machinery with `poise-health` is reasonable; sharing the request and
response type is not, and would couple eligibility to ranking in exactly the way
[architecture](architecture.md) forbids.

**The fallback is uniform random, and it triggers on occupancy rather than
emptiness.** That settles its behavior but not its placement. A type parameter
defaulting to a probe-free policy keeps `Prequal` total and keeps the fallback
observable through `regime`; explicit caller composition keeps the type simpler
but makes silent fallback easy to write by accident. The former is proposed, on
the standing requirement that outcomes stay distinguishable.

## What this chapter is not proposing

Prequal, as the paper describes it, is a load balancer: probe scheduling tied to
request rate, a reuse budget derived from fleet size, degradation control paced
per query, sinkhole heuristics, and a synchronous mode that puts probing on the
request path. Poise is not that, and reading the chapter end to end it would be
easy to conclude otherwise, because describing a system faithfully means
describing all of it.

The line is the same one the rest of the workspace draws. Poise ships
primitives: a bounded store with a retention contract, and a selection rule that
is a pure function of what it is handed. A balancer is something an application
composes *from* those, and several questions that look open from inside the
paper are already answered by that boundary.

**Membership is the caller's.** `ProbePool` is generic over identity and stores
what it is given. It is not authoritative over eligibility, and `decide` takes
the ranking function rather than returning the slice, so the only code that sees
both the retained observations and the current candidate set is the caller's
selector — which is exactly where an entry naming a departed replica gets
dropped. Whether identity means a name or a discovery generation is a decision
for whoever owns the membership, and `poise-discovery` already owns it. The pool
needs no opinion, and giving it one would put a second membership model in the
workspace.

**Rates are the caller's.** A removal rate denominated in queries cannot live in
something that never sees a query. The same is true of the reuse budget, which
the paper derives from probing rate, removal rate, pool size, and replica count:
the pool knows one of those four. It exposes the bound; whatever counts requests
chooses it.

**Failure detection is health's.** Sinkholing — a replica failing fast enough to
look fast — is real, and it is not a probe-pool problem. Passive health, circuit
breaking, and outcome windows already observe error rates, and probes are
explicitly not authoritative over eligibility, so a sinkholing replica should be
excluded before ranking ever sees it. Composing that correctly is the
application's job, and a primitive that quietly filtered on error rate would be
duplicating a subsystem that already exists.

**Scheduling is a runtime's.** Probe issuance, its coupling to request rate, and
the synchronous variant all need a scheduler. `poise-core` has none and must not
acquire one. Those belong in `poise-tokio` or above, and the rule must stay a
function of a pool it did not fill.

What remains in scope is small and concrete: the hot-cold rule as a selection
policy alongside the other thirteen, and the retention mechanics the rule
depends on.

## Selecting against a candidate set

Membership being the caller's does not mean the composition should be the
caller's to get right unaided. `decide_at` hands a ranking function every
retained observation, including ones naming replicas that have since drained or
left, and filtering them is documented as the caller's responsibility. That
responsibility is silently omissible, and omitting it routes traffic to a
departed replica — the exact hazard the retention contract warns about, with
nothing in the signature to prevent it. Every caller then writes the same
filter, and afterwards searches the candidate slice to turn the returned
observation back into something dispatchable.

`ProbePool::decide_among_at` takes the candidate slice and does that work. It
intersects unexpired observations with the candidates by identity, drops any
that are not eligible, and offers the ranking function only what is left. The
decision carries the chosen candidate's position, so nothing maps back:

```rust
let decision = pool.decide_among(&candidates, |observed| {
    observed
        .iter()
        .enumerate()
        .min_by_key(|(_, entry)| entry.requests_in_flight())
        .map(|(index, _)| index)
})?;
dispatch(&candidates[decision.selection().index()]);
```

The closure now expresses only the ranking, which is the part that is actually
the policy.

This is not the pool acquiring a membership model, which the section above rules
out. It caches no candidate set, tracks no generation, and defines no
eligibility of its own — it asks `Candidate::is_eligible`, which the caller's
type implements, and compares against the slice handed to it at the moment of
the call. It owns no opinion; it declines to offer a footgun.

That also settles what looked like an open question about keying. Identity is
compared at decision time against the slice the caller passed, so an entry
naming a replica that has left is invisible without any keying scheme,
generation tracking, or staleness rule. `decide_at` remains for callers who want
the unfiltered view.

Cold start stays the caller's to handle, deliberately. The paper falls back
below an occupancy of two rather than at emptiness, and `len_at` reports
occupancy so a caller can implement that threshold; the pool does not choose one.

## The one open API question

Degradation is a retention property, so it belongs to the pool if it belongs
anywhere here — selection consumes the observations reporting idle replicas
first, and what accumulates is a pool describing the fleet as busier than it is.
Removing the worst entry needs a ranking, which the pool does not have and
should not acquire.

The shape that fits is the one `decide_at` and `decide_among_at` established: a
removal entry point taking the caller's ranking, applied in reverse, under the
same lock. That is the only remaining addition to the pool's surface this
chapter proposes. Pacing it stays outside, for the reason above.

## References

- [Load is not what you should balance: Introducing Prequal](https://www.usenix.org/conference/nsdi24/presentation/wydrowski),
  Wydrowski, Kleinberg, Rumble, and Archer, USENIX NSDI 2024.
- [Architecture](architecture.md) for the boundary this policy must respect.
- [Load and feedback](load-and-feedback.md) for the tracker contract it extends.
