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
accrual, and coherent rolling-window snapshots during concurrent record/clear
operations.
