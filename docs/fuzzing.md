# Fuzzing and concurrency models

Poise has three structure-aware libFuzzer targets:

- `policy_state_machine` mutates backend order, health, weight, and load while
  exercising general, load-aware, affinity, and stateful policies;
- `topology_state_machine` generates priority, panic, and locality scenarios
  and checks every successful decision against candidate metadata;
- `probe_pool` interleaves probe recording, selection, rejection, and clock
  advance against the retention contract, checking that capacity holds, that no
  expired observation informs a decision, that every decision charges exactly
  one use, and that a rejected decision spends none.

Build the targets without executing them:

```console
cargo +nightly fuzz build
```

The local smoke wrapper is intentionally resource constrained:

```console
scripts/fuzz-smoke.sh
```

Each target defaults to 1,000 executions, a 256-byte maximum input, and a 128
MiB libFuzzer RSS watchdog. The wrapper additionally refuses to run without a
systemd user manager that can place it in a cgroup with a 25% CPU quota, 192 MiB
hard memory limit, no swap, low scheduler priority, and a 30-second process
deadline. Set `POISE_FUZZ_RUNS` only when the machine has enough headroom.
Sustained fuzzing belongs on an isolated worker with externally enforced CPU
and memory limits; do not turn the local smoke wrapper into an unbounded time
campaign.

AddressSanitizer remains enabled. LeakSanitizer alone is disabled because it
cannot operate in ptrace-constrained containers.

The lock-free `InFlight` tracker and shared health state machines are compiled
against Loom's synchronization types and checked across modeled scheduler and
memory-order interleavings:

```console
scripts/model-check.sh
```

This is a separate build under `cfg(loom)`; ordinary builds continue to use
`std::sync`. The models verify in-flight limit and balance laws, single active
probe admission, forced-status generation invalidation, passive-circuit failure
accrual, bounded probe reuse under concurrent selection, and coherent
rolling-window snapshots during concurrent record/clear operations.

## Model-checking bounds

Loom explores a model exhaustively, so its cost is combinatorial in the threads
and shared operations a model contains. Today's ten models all complete well
inside loom's default ceiling, but the failure mode of adding one more
concurrent step is a state-space explosion rather than a gradual slowdown. The
wrapper bounds that the same way the mutation gate is bounded, and for the same
reason: an unbounded verification job is capable of taking its machine down.

- **Build and test parallelism are capped.** The test harness otherwise runs one
  test per core, so peak memory is per-model cost times core count. The wrapper
  caps concurrent models with `RUST_TEST_THREADS` and rustc parallelism with
  `CARGO_BUILD_JOBS`.
- **`LOOM_MAX_BRANCHES` is set explicitly** rather than left implicit. Exceeding
  it aborts the model loudly, so this bound cannot silently shrink coverage: a
  model that outgrows it fails until someone raises it deliberately.
- **`LOOM_MAX_PREEMPTIONS` is deliberately not set.** Bounding preemptions makes
  exploration cheaper by making it partial, and it does so silently. That trades
  away the property this gate exists to establish.
- **Memory, CPU, and wall clock are capped** inside a transient systemd scope
  with swap disabled. The wall clock interrupts first, escalates to `SIGKILL`,
  and is backed by a slightly later `RuntimeMaxSec` on the scope, which stops
  the whole cgroup rather than only the direct child.

`POISE_LOOM_BUILD_JOBS`, `POISE_LOOM_TEST_THREADS`, `POISE_LOOM_MEMORY_MAX`, and
`POISE_LOOM_TIMEOUT` override the defaults.

Isolation is preferred but not required, and the three wrappers differ on this
deliberately. `scripts/model-check.sh` and `scripts/mutants-core.sh` probe for a
usable scope and fall back to their remaining bounds with a warning when there
is none, because both run in CI and `systemd-run --user` needs a session bus
that a runner may not provide — and a CI worker is already an externally limited
sandbox. Set `POISE_REQUIRE_ISOLATION=1` to turn a missing cage into a hard
failure for those two.

`scripts/fuzz-smoke.sh` has no fallback and exits 78 without `systemd-run`. That
is not an inconsistency: nothing in CI runs it, and a fuzz target is designed to
run until something breaks, so an unbounded local fuzz run has no natural end to
degrade toward.
