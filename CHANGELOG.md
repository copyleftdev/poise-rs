# Changelog

All notable changes to Poise will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this workspace follows [Semantic Versioning](https://semver.org/) within the
usual pre-1.0 compatibility expectations.

Project development can be supported through
[TokenTip](https://tokentip.to/@copyleftdev).

## [Unreleased]

### Added

- `ProbePool`, a bounded and self-expiring pool of out-of-band replica
  observations, with `ProbePoolConfig`, `ProbeReading`, `ProbeEntry`, and typed
  configuration and decision errors. Selection and reuse charging happen under
  one lock so concurrent selectors cannot overspend a single observation.
- Loom models over concurrent probe-pool consumption and a `probe_pool` fuzz
  target covering record, expire, and select interleavings.
- A [Prequal proposal](docs/prequal.md) chapter stating the probe-based
  selection contract ahead of the policy itself, with the originating paper's
  operating points recorded, three of its open questions resolved against them,
  and degradation control identified as an obligation the pool does not yet
  meet.

### Changed

- `scripts/mutants-core.sh` enforces its own resource bounds instead of relying
  on the caller to pass them: one worker by default under a four-worker ceiling,
  capped build parallelism, a memory ceiling with swap disabled, a CPU quota, a
  wall-clock backstop, and scratch directories kept off RAM-backed filesystems.
  An unbounded campaign is capable of taking a workstation down; nested
  cargo-mutants and cargo parallelism multiply, and the per-worker tree copies
  land in `TMPDIR`, which is a tmpfs on many systems.
- `scripts/model-check.sh` bounds Loom the same way, through a shared
  `scripts/lib/resource-bounds.sh`: capped concurrent models and build
  parallelism, a memory ceiling with swap disabled, a CPU quota, a wall-clock
  backstop, and an explicit `LOOM_MAX_BRANCHES`. Exceeding the branch ceiling
  fails loudly, so the bound cannot silently shrink coverage; preemption
  bounding, which would, is deliberately left unset. Both wrappers prefer a
  systemd scope and fall back to their remaining bounds with a warning where one
  cannot be created, since CI runners have no user session bus.

## [0.1.1] - 2026-08-05

### Documentation

- Added a durable project-support link to package READMEs, crate-level rustdoc,
  and the engineering book.

## [0.1.0] - 2026-08-05

### Added

- Initial policy, affinity, topology, health, discovery, Tower, Tokio, and
  observability foundations.
- Property, mutation, fuzz, Loom, MSRV, and deterministic replay verification.
- Bounded Invariant Orrery capability showcase and live CI evidence contract.
