# Contributing to Poise

Poise welcomes focused bug reports, design discussion, documentation, tests,
and implementation work. Load-balancing changes can affect correctness under
churn, concurrency, and failure, so pull requests are expected to explain the
behavioral contract they preserve or introduce.

## Before opening a change

- Read [Architecture](docs/architecture.md) and the contract closest to the
  affected subsystem.
- Search existing issues and discussions before proposing a new public API.
- Use a discussion for a cross-crate design, policy semantic change, or new
  dependency before investing in a large patch.
- Keep runtime dependencies out of `poise-core`, `poise-discovery`, and
  `poise-health`.

## Development setup

Poise requires Rust 1.85 or newer. Install the stable toolchain with Clippy and
rustfmt, then install the repository-owned validation hook:

```console
rustup toolchain install stable --component clippy,rustfmt
scripts/install-hooks.sh
```

The hook checks release metadata and shared workspace versions. It does not run
fuzzing, mutation testing, or a full compile.

## Fast validation

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo +1.85.0 test --workspace --all-features
```

Documentation changes should also pass:

```console
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo install mdbook --version 0.5.2 --locked
scripts/build-book.sh
```

`docs/SUMMARY.md` is the book's navigation contract. Every Markdown chapter in
`docs/` must appear there, use one prose-level title, and keep local links
resolvable. The build script enforces those rules before rendering the same
`site/book/` artifact deployed by GitHub Pages.

## Choosing the right proof

Add the strongest applicable evidence:

| Change | Expected evidence |
| --- | --- |
| Example or edge behavior | Unit test and rustdoc where public |
| General policy law | Proptest law with bounded shrinking |
| Seeded/random policy | Exact deterministic replay plus distribution bounds |
| Affinity or membership churn | Addition, removal, reordering, and disruption properties |
| Atomic or concurrent state | Loom model plus an ordinary concurrency test |
| Mutation survivor | Focused contract test that fails for the mutation |
| Hot-path algorithm | Criterion comparison and complexity documentation |

Property tests use a fixed CI seed while still exploring fresh cases locally.
Commit new `.regressions` files: they replay minimized failures before novel
inputs.

Run Loom explicitly:

```console
scripts/model-check.sh
```

Mutation campaigns are maintainer/deep-CI work and default to one worker:

```console
scripts/mutants-core.sh --jobs 1
```

Do not launch fuzz targets without the time, memory, and corpus limits in
[Fuzzing and concurrency](docs/fuzzing.md).

## API and implementation expectations

- Keep empty, ineligible, overflow, and invalid-configuration outcomes
  distinguishable.
- Preserve candidate identity across reorder and membership churn where the
  policy contract requires it.
- Keep cached rebuilds transactional: invalid replacement state must not damage
  the live generation.
- Make cancellation behavior explicit and RAII-safe.
- Avoid endpoint-derived metric dimensions and unbounded internal collections.
- Document time, memory, allocation, tie, overflow, and fallback behavior for a
  new stable policy.
- Do not introduce `unsafe`; the workspace forbids it.

## Commits and pull requests

Use Conventional Commits:

```text
feat(core): add retry exclusion contract
fix(health): preserve probe epoch across forced status
docs: explain locality spillover arithmetic
test(core): catch ring cursor mutation
```

Add `!` or a `BREAKING CHANGE:` footer for an intentional breaking change.
Release automation uses this information to propose SemVer bumps and changelog
entries.

Pull requests should remain reviewable and include:

- the user-visible behavior or defect;
- why the selected contract is correct;
- the tests or models that would fail if it regressed;
- compatibility, performance, allocation, and observability impact;
- documentation updates for public behavior.

## Version changes

Normal version bumps come from the automated release PR. For an explicit
maintainer override:

```console
node scripts/bump-version.mjs patch
node scripts/bump-version.mjs 0.2.0
```

Review `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` together. Never edit one
member version independently. See [Release engineering](docs/releasing.md).

## Reporting security problems

Follow [SECURITY.md](SECURITY.md). Do not put vulnerability details in an issue,
discussion, or pull request.
