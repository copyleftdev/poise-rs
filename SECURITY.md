# Security policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting flow from the repository's
**Security** tab. Include:

- the affected crate, version, feature set, and Rust toolchain;
- the violated safety, routing, isolation, or availability property;
- a minimal reproducer or failing test when possible;
- expected impact and any known mitigations;
- whether the report is under active exploitation or public elsewhere.

Do not open a public issue, discussion, or pull request containing vulnerability
details. Maintainers will acknowledge a complete report within seven days and
coordinate validation, remediation, advisory publication, and credit.

## Supported versions

Poise has not published its first public crate release. Until then, security
fixes target the current `main` branch. After publication, this table will name
the maintained release lines and their support windows.

## Scope

Security-sensitive behavior includes, but is not limited to:

- routing to an ineligible, draining, unhealthy, or opted-out backend;
- arithmetic that bypasses load, weight, priority, or locality bounds;
- state corruption during membership reconciliation;
- readiness or cancellation behavior that leaks capacity;
- unbounded resource growth reachable through public input;
- telemetry cardinality that can be controlled by untrusted endpoint identity;
- dependency or release-pipeline compromise.

The workspace forbids unsafe Rust, but logic, denial-of-service, and
supply-chain defects remain in scope.
