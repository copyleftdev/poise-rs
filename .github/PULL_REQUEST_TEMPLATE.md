## What changed

<!-- Describe the user-visible or maintainer-visible outcome. -->

## Contract and rationale

<!-- Explain the invariant, compatibility rule, or failure behavior involved. -->

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`
- [ ] Rust 1.85 checked when dependencies or public API changed
- [ ] Property or Loom evidence added where applicable
- [ ] Public behavior and changelog impact documented

## Operational impact

<!-- Cover performance, allocation, cancellation, observability, and rollout concerns. -->
