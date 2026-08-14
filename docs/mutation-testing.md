# Mutation testing

Poise requires every terminating, viable `poise-core` mutation to be caught.
This is a behavioral gate, not a coverage percentage target: one unexplained
survivor fails the campaign.

The campaign is reproducible with cargo-mutants 27.0.0:

```console
scripts/mutants-core.sh
```

The wrapper enables all `poise-core` features and fixes the Proptest seed.
Single-worker execution avoids sharing incremental compiler state between
mutant copies, and is now the enforced default rather than a flag to remember.
`.cargo/mutants.toml` owns timeouts and a small, reviewed list of equivalent
mutations. Each exclusion has an adjacent proof explaining why the rewrite
cannot change observable behavior. Do not add score-based or broad file
exclusions to make a campaign green.

## Resource bounds

A campaign rebuilds and retests the crate once per mutant, 705 times over. It is
the most resource-hostile thing in this repository, and an unbounded run is
capable of taking a workstation down hard enough to corrupt the git object
store. The wrapper enforces its own limits; none of them depend on the caller
passing a flag.

- **Nested parallelism is capped.** cargo-mutants runs `--jobs` cargo
  invocations, and each one spawns its own `CARGO_BUILD_JOBS` rustc processes.
  The live process count is the *product* of the two, and neither factor bounds
  the other, so their defaults multiply to `nproc` squared on a many-core
  machine. The wrapper pins one worker and eight build jobs, and clamps any
  larger `--jobs` request to a four-worker ceiling.
- **Scratch directories stay off RAM-backed filesystems.** cargo-mutants copies
  the source tree and the whole target directory per worker — currently 555 MB
  each — into `TMPDIR`. Where `/tmp` is a tmpfs, that default puts every copy in
  RAM and pushes roughly 390 GB of copy traffic through unreclaimable pages over
  a full campaign. The wrapper defaults `TMPDIR` to `target/mutants-scratch` and
  refuses to start if that path is on `tmpfs` or `ramfs`.
- **Memory is capped with swap disabled.** The campaign runs in a transient
  systemd scope with `MemoryMax` and `MemorySwapMax=0`. Measured peak is 1.4 GB
  per worker against a 4 GB allowance. Disabling swap is the load-bearing half:
  without it the kernel answers memory pressure by thrashing, which is what
  makes the whole machine unresponsive rather than just failing the campaign.
- **CPU is capped and the campaign is niced**, so an interactive session stays
  responsive. Isolation is preferred but not required: `systemd-run --user`
  needs a session bus that CI runners generally lack, so the wrapper probes for
  a usable scope and falls back to its remaining bounds with a warning when
  there is none. Set `POISE_REQUIRE_ISOLATION=1` to make a missing cage a hard
  failure instead.
- **A wall-clock backstop stops a wedged campaign.** A full run takes roughly
  twenty minutes; the default ceiling is ninety. It interrupts first so cargo
  can report, then escalates to `SIGKILL`, and where a systemd scope exists a
  slightly later `RuntimeMaxSec` stops the entire cgroup — which is what
  actually covers descendants, since `timeout` only ever signals its direct
  child.

Exceeding the wall clock or the memory ceiling is reported as itself rather than
as a mutation failure, so a limit that has genuinely become too tight for a
growing crate is visible as a limit. `POISE_MUTANTS_JOBS`,
`POISE_MUTANTS_MAX_JOBS`, `POISE_MUTANTS_BUILD_JOBS`,
`POISE_MUTANTS_MEMORY_PER_JOB_MB`, `POISE_MUTANTS_TIMEOUT`, and
`POISE_MUTANTS_SCRATCH` override the defaults. Raise one only after confirming
the growth that needs it; the previous freeze is what these numbers are for.

For a faster follow-up after adding tests, preserve `mutants.out` and run:

```console
scripts/mutants-core.sh --iterate
```

`mutants.out` is a local report and is intentionally ignored. Review its four
outcome classes separately:

- **caught** means a test failed under the mutation;
- **missed** means a terminating viable mutation survived and fails the gate;
- **timeout** means a mutation changed the normally sub-second suite into a
  watchdog-detected hang;
- **unviable** means the mutation did not compile and is not scored.

The wrapper accepts cargo-mutants' timeout-only exit status only when
`missed.txt` exists and is empty. This is necessary for loop-control mutations
in Maglev, ring hash, and the lock-free in-flight limiter: their defect is the
nontermination detected by the watchdog. Baseline failure, any surviving
terminating mutant, and all other tool failures remain nonzero.

## Baseline

The 2026-08-04 initial inventory generated 656 mutation sites. Before this
hardening pass, 417 were caught, 94 were missed, 23 timed out, and 122 were
unviable: 81.6% of completed viable mutations were caught.

The hardened source and reviewed exclusions currently examine 705 sites. A
complete campaign caught 528, missed zero, found 154 unviable, and detected 23
nonterminating mutants with the watchdog. The hardening work converted 82 of
the original survivors into caught tests, proved 11 equivalent and documented
them, and removed one performance-only Maglev cursor mutation by making
overflow handling explicit. The watchdog outcomes remain deliberately visible
rather than excluded.

Re-run the complete campaign whenever selection arithmetic, health/topology
boundaries, hashing, cached membership, or load-tracker concurrency changes.
Update this baseline only from a clean unmutated test run, and never describe a
partial or filtered campaign as the new baseline.
