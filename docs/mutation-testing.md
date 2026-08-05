# Mutation testing

Poise requires every terminating, viable `poise-core` mutation to be caught.
This is a behavioral gate, not a coverage percentage target: one unexplained
survivor fails the campaign.

The campaign is reproducible with cargo-mutants 27.0.0:

```console
scripts/mutants-core.sh --jobs 1
```

The wrapper enables all `poise-core` features and fixes the Proptest seed.
Single-worker execution avoids sharing incremental compiler state between
mutant copies. `.cargo/mutants.toml` owns timeouts and a small, reviewed list of
equivalent mutations. Each exclusion has an adjacent proof explaining why the
rewrite cannot change observable behavior. Do not add score-based or broad
file exclusions to make a campaign green.

For a faster follow-up after adding tests, preserve `mutants.out` and run:

```console
scripts/mutants-core.sh --iterate --jobs 1
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

The hardened source and reviewed exclusions currently examine 642 sites. A
complete campaign caught 497, missed zero, found 122 unviable, and detected 23
nonterminating mutants with the watchdog. The hardening work converted 82 of
the original survivors into caught tests, proved 11 equivalent and documented
them, and removed one performance-only Maglev cursor mutation by making
overflow handling explicit. The watchdog outcomes remain deliberately visible
rather than excluded.

Re-run the complete campaign whenever selection arithmetic, health/topology
boundaries, hashing, cached membership, or load-tracker concurrency changes.
Update this baseline only from a clean unmutated test run, and never describe a
partial or filtered campaign as the new baseline.
