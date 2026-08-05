# Security model

Poise processes control-plane membership, routing keys, weights, health
outcomes, and service errors. Even without unsafe Rust or direct network I/O,
those inputs can affect availability, resource use, and traffic isolation.

## Trust boundaries

| Input | Typical source | Risk |
| --- | --- | --- |
| Membership identity | DNS, xDS, Kubernetes, config | Collision, churn, duplicate identity |
| Weight and topology | Operator or control plane | Traffic concentration, overflow, failed isolation |
| Affinity key | Request data | Hot-key amplification, privacy leakage |
| Health result | Probe or request classifier | False ejection or false recovery |
| Candidate load | Shared tracker or application | Stale or incomparable decisions |
| Custom policy index | Application code | Out-of-bounds dispatch |
| Observability fields | Application integration | Cardinality and sensitive-data exposure |

Validate external inputs before constructing policy configuration. Typed
constructors reject zero weights, invalid percentages, invalid prime/table
sizes, and arithmetic overflow; they cannot determine whether a syntactically
valid value is operationally safe.

## Memory safety

The workspace forbids unsafe Rust through a workspace lint. This reduces one
class of defects; it does not eliminate denial of service, logic error,
dependency compromise, starvation, or incorrect operational configuration.

Tower validates custom-policy indices before endpoint access. Snapshot and
policy cache updates are staged so a partial failure cannot expose mixed state.

## Algorithmic denial of service

Candidate count, ring points, Maglev table size, and outcome-window capacity
must be bounded by configuration. Do not let one request choose these values.

Affinity keys can be attacker-controlled. A malicious key distribution may
concentrate traffic even when the hashing implementation is deterministic and
collision-safe. Bounded-load affinity limits prospective capacity but still
requires atomic admission.

Complete hash collisions have defined deterministic behavior; they do not
panic. Deterministic FNV-based hashing is used for stable placement, not as a
cryptographic MAC or untrusted hash-table defense.

## Topology isolation

Priority panic can deliberately broaden health eligibility. Review
`PanicMode` as a security and isolation control:

- fail-closed preserves exclusion at the cost of availability;
- broader panic can route to unhealthy capacity;
- draining and operator opt-out remain excluded.

Do not encode tenant authorization as a load-balancer preference. A policy
selects among candidates it is given; the caller must construct an authorized
candidate scope first.

## Sensitive data

Built-in metrics exclude backend IDs, affinity keys, endpoint indices, policy
names, and error strings from labels. Optional tracing can expose numeric
indices and fixed classifications.

Applications adding logs or traces should consider:

- affinity keys may contain user or tenant identifiers;
- backend identity may reveal internal topology;
- error text may contain protocol or customer data;
- full discovery snapshots can expose infrastructure inventory.

Prefer stable opaque IDs, sampling, and access-controlled diagnostic sinks.

## Supply-chain controls

The repository enforces:

- Cargo.lock in CI;
- cargo-deny advisories, licenses, bans, and source policy;
- an independent RustSec advisory check;
- immutable GitHub Action commit pins;
- Dependabot updates for Cargo and Actions;
- protected release environments;
- OIDC trusted publishing after the first crate bootstrap;
- an explicit release kill switch.

The initial crates.io publication is exceptional because trusted publishing
cannot claim a new crate name. Its short-lived token belongs only in the
protected bootstrap environment and must be deleted afterward.

## Vulnerability reporting

Do not open a public issue for a suspected vulnerability. Follow the private
process in the repository [security policy](https://github.com/copyleftdev/poise-rs/security/policy).

A useful report includes the affected crate and revision, threat model,
minimal reproduction, observed impact, and whether the issue requires
attacker-controlled membership, keys, weights, timing, or application code.

## Security review checklist

- Are all externally derived sizes and weights bounded?
- Can an attacker force expensive membership rebuilds per request?
- Are affinity keys canonical, opaque, and free of secrets?
- Does topology panic cross an isolation boundary?
- Are retry and hedge counts bounded?
- Can cancellation strand capacity?
- Can an old generation update current health or load?
- Are diagnostic labels and logs cardinality-bounded?
- Are release dependencies pinned and reviewed?
