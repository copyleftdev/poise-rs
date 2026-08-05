# Getting started

Poise is split into six crates so the deterministic policy core does not pull an
async runtime, Tower, tracing, or a discovery implementation into every build.

## Choose only the layers you need

| Crate | Add it when you need |
| --- | --- |
| `poise-core` | Candidate contracts, policies, affinity, topology, or load trackers |
| `poise-discovery` | Versioned membership snapshots and graceful draining |
| `poise-health` | Active health, passive circuits, outcome windows, or outlier analysis |
| `poise-tower` | Readiness-correct Tower dispatch and snapshot reconciliation |
| `poise-tokio` | Tokio timers for probes or async snapshot waits |
| `poise-observe` | Fixed-cardinality counters and optional tracing |

There is no umbrella crate. This is intentional dependency hygiene, not an
unfinished convenience API.

## Install from a pinned revision

Before the first crates.io publication, use a Git dependency pinned to a commit:

```toml
[dependencies]
poise-core = { git = "https://github.com/copyleftdev/poise-rs", rev = "<reviewed-40-character-commit>" }
```

For a local workspace integration:

```toml
[dependencies]
poise-core = { path = "../poise-rs/crates/poise-core" }
```

Replace the placeholder with a full commit hash you have reviewed. Do not
depend on an unpinned branch in a release build. Poise is pre-1.0 and the
default branch is allowed to advance.

## Make a first selection

```rust
use poise_core::{Backend, Policy, policy::RoundRobin};

let backends = [Backend::new("alpha"), Backend::new("beta")];
let mut policy = RoundRobin::new();

let selection = policy.pick(&backends, &())?;
let backend = &backends[selection.index()];

assert_eq!(backend.id(), &"alpha");
# Ok::<(), poise_core::PickError>(())
```

`Policy::pick` returns an index rather than cloning or borrowing a backend. The
caller retains ownership of membership and can attach protocol-specific state
outside the policy.

## Express eligibility and capacity

```rust
use poise_core::{Backend, Policy, Status, Weight, policy::WeightedRandom};

let backends = [
    Backend::new("large").with_weight(Weight::new(4)?),
    Backend::new("small").with_weight(Weight::new(1)?),
    Backend::new("draining").with_status(Status::Draining),
];
let mut policy = WeightedRandom::seeded(7);

for _ in 0..100 {
    let selected = policy.pick(&backends, &())?;
    assert_ne!(backends[selected.index()].id(), &"draining");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Weights are nonzero integers. Status is explicit. Every general policy excludes
draining and unavailable candidates, while topology panic behavior documents
its narrower exceptions.

## Handle the no-candidate cases

```rust
use poise_core::{Backend, PickError, Policy, Status, policy::RoundRobin};

let mut policy = RoundRobin::new();
let empty: [Backend<&str>; 0] = [];
assert_eq!(policy.pick(&empty, &()), Err(PickError::Empty));

let unavailable = [Backend::new("alpha").with_status(Status::Unavailable)];
assert_eq!(
    policy.pick(&unavailable, &()),
    Err(PickError::NoEligibleCandidates)
);
```

Do not collapse these errors at the policy boundary:

- `Empty` usually means discovery has no membership.
- `NoEligibleCandidates` means membership exists but health, draining, or
  operator state excluded it.

Those conditions often deserve different retry, fallback, and alert behavior.

## Make stochastic behavior reproducible

Randomized policies offer `seeded` constructors and caller-provided RNGs.
Production systems may seed from entropy; tests and simulations should use a
recorded seed:

```rust
use poise_core::{Backend, Policy, policy::Random};

let backends = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
let mut left = Random::seeded(0x5eed);
let mut right = Random::seeded(0x5eed);

for _ in 0..32 {
    assert_eq!(left.pick(&backends, &())?, right.pick(&backends, &())?);
}
# Ok::<(), poise_core::PickError>(())
```

Reproducibility matters for incident reconstruction, property tests, and
simulation comparisons. It does not make separate policy instances share state.

## Continue from here

- [Choose a policy](choosing-a-policy.md) compares selection goals and costs.
- [Compose the system](composition.md) places health, discovery, Tower, and
  observation around selection.
- [Failure semantics](failure-semantics.md) explains errors and lifecycle
  outcomes that should remain distinct.
