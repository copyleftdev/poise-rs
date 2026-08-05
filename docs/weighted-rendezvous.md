# Weighted rendezvous contract

`WeightedRendezvous` provides capacity-proportional, key-affine selection with
minimal disruption. It follows the logarithmic weighted-HRW family described by
Schindelhauer and Schomaker's
[Weighted Distributed Hash Tables](https://doi.org/10.1145/1073970.1074008)
and the weighted score summarized by the
[IETF weighted-HRW draft](https://www.ietf.org/archive/id/draft-ietf-bess-weighted-hrw-02.html#section-4).

## Selection

For each eligible candidate, Poise hashes the request context and stable backend
identity, maps that hash to `U` in `(0, 1]`, and computes:

```text
race = -ln(U) / weight
```

The smallest race wins. This is equivalent to maximizing
`-weight / ln(U)`. Positive integer weights therefore define relative expected
assignment shares: weights `1, 3, 6` target 10%, 30%, and 60% of a sufficiently
large independent key population.

Selection is `O(n)` time, `O(1)` additional memory, and allocation-free. The
policy borrows the candidate slice and returns only a `Selection` index.

Empty slices return `PickError::Empty`. Non-empty slices without an eligible
candidate return `PickError::NoEligibleCandidates`. Draining and unavailable
candidates are excluded before hashing.

## Minimal disruption

Every candidate score depends only on the request, that candidate's identity,
and that candidate's own weight. Consequently:

- removing a backend only remaps keys previously assigned to it;
- adding a backend only moves keys that the new backend wins;
- changing one weight never moves a key directly between two unchanged
  backends;
- reordering a unique-identity candidate slice changes indices but not winning
  identities.

These guarantees assume the request hash, candidate identity, candidate weight,
eligibility, and hash builder remain unchanged where stated.

## Deterministic hash pipeline

The default pipeline is part of the compatibility contract:

1. FNV-1a hashes domain-separated request and identity values.
2. `mix64` applies a bijective SplitMix64 avalanche finalizer. This prevents
   nearby structured FNV inputs from retaining correlations that the weighted
   logarithm would amplify.
3. The high 53 bits plus one form an exactly representable sample in
   `1..=2^53`, which maps to `U` in `(0, 1]`.
4. A fixed 13-term range-reduced series computes `ln(U)` using specified basic
   floating-point operations rather than platform `libm`.
5. The complete 64-bit mixed hash breaks equal transformed-score ties. This
   makes equal-weight selection exactly match ordinary `Rendezvous`.

The avalanche finalizer is shared by ordinary and weighted rendezvous. It
improves distribution for structured keys while leaving the public FNV-1a byte
algorithm itself unchanged.

`FnvBuildHasher` is stable and inexpensive, not collision-resistant. When keys
or backend identities are adversarial, callers should use `with_hasher` and a
builder appropriate to their threat model. Reproducible assignment then also
depends on that builder being deterministic and identically configured across
participants.

## Duplicate identities

Eligible identities should be unique. Enforcing this inside every selection
would require extra memory or quadratic work, so the allocation-free policy
defines duplicates rather than rejecting them:

- duplicate identities receive the same random draw;
- the duplicate with greater weight wins;
- equal-weight duplicates resolve to the earlier slice entry.

Control planes that require strict identity uniqueness should validate their
membership snapshot once, before it reaches the request path.
