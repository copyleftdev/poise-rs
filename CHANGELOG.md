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
  selection contract ahead of the policy itself.

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
