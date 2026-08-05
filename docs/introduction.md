# Poise documentation

Poise is a family of composable, runtime-independent load-balancing primitives
for Rust. It is designed for engineers who need to explain why a backend was
eligible, why a policy selected it, what happened during dispatch, and how the
result changes future decisions.

The library does not treat load balancing as one algorithm. It models a control
loop:

```text
discovery → membership → eligibility → selection → dispatch → feedback
                               ↑                        │
                               └── health and load ─────┘
```

Each arrow is a contract. Keeping those contracts separate lets applications
change discovery, policy, runtime, health strategy, or telemetry without
replacing an opaque all-in-one balancer.

## What Poise guarantees

Poise makes narrow guarantees that compose:

- a successful selection always identifies an eligible, in-bounds candidate;
- empty membership and non-empty but ineligible membership remain distinct;
- seeded stochastic policies replay exactly;
- keyed policies use documented deterministic hashing;
- cached membership state is rebuilt transactionally;
- Tower readiness is retained between selection and dispatch;
- completion and cancellation update load through RAII guards;
- discovery readers observe immutable, monotonically versioned snapshots;
- built-in metrics have fixed cardinality independent of backend and request
  diversity.

Those are behavioral contracts rather than aspirations. The test surface
combines examples, generated laws, mutation testing, and exhaustive scheduler
models. The [live verification record](https://copyleftdev.github.io/poise-rs/)
links every published result to the GitHub Actions run that produced it.

## What Poise does not own

Poise is not a reverse proxy, HTTP client, service mesh, DNS resolver, retry
engine, or orchestrator SDK. It does not spawn a runtime from the core, own
connections, select retry semantics, or create unbounded telemetry labels.

That restraint is useful. A proxy can use Poise without adopting its protocol
stack. A library can expose policies without imposing Tokio. A control plane can
publish snapshots without owning dispatch.

## How to read this book

If you are evaluating the crate, begin with [Choose a policy](choosing-a-policy.md)
and [Compose the system](composition.md). They explain the decision surface and
where each crate belongs.

If you are integrating Poise, read [Failure semantics](failure-semantics.md),
[Tower dispatch](tower.md), and [Operating Poise](operations.md) before adding
retries or health automation.

If you are changing a policy, its focused contract and the
[testing strategy](testing.md) are part of the public API. Arithmetic, hashing,
membership invalidation, and concurrency changes require evidence at the
strongest applicable verification layer.

## Maturity

Poise is pre-1.0. Its contracts are deliberately explicit and heavily tested,
but new capabilities can still reshape APIs. All six workspace crates share one
version and one release process. The first crates.io publication remains behind
a protected manual bootstrap; until then, use a pinned Git revision when
evaluating the library.

## Documentation conventions

The book uses four kinds of statements:

- **Invariant** — behavior callers may rely on.
- **Tradeoff** — a cost or limitation that influences policy choice.
- **Operational rule** — a condition production integrations should enforce.
- **Non-goal** — behavior intentionally left to the application.

Examples prefer concrete failure handling over happy-path-only snippets.
Complexity statements describe the current implementation, not an imagined
future optimization.
