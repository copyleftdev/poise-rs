# Release engineering

Poise uses one version across six crates and publishes them in dependency order.
Normal releases are pull-request driven; direct `cargo publish` from a maintainer
laptop is an emergency procedure, not the default.

## Current hard gates

The repository, license, and Cargo metadata are finalized. The first crates.io
publication remains blocked until all of these are true:

1. All six crate names are checked immediately before publication.
2. GitHub Actions, protected environments, required reviews, and private
   vulnerability reporting are enabled.
3. The complete CI, MSRV, Loom, package, and mutation gates are green on the
   exact release commit.
4. A maintainer supplies a short-lived, least-privilege bootstrap token and
   enters the workflow's exact confirmation phrase.

`node scripts/check-release-metadata.mjs --publishable` enforces the mechanical
portion of this list.

## Cargo metadata

The shared package identity is defined once:

```toml
[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
publish = true
license = "MIT OR Apache-2.0"
repository = "https://github.com/copyleftdev/poise-rs"
homepage = "https://copyleftdev.github.io/poise-rs/"
```

Every member inherits `license`, `repository`, and `homepage`. The release
metadata gate rejects drift between the workspace and member manifests.

## Repository bootstrap

Authenticate the intended GitHub owner, initialize the real checkout, then run:

```console
gh auth login
scripts/bootstrap-github.sh copyleftdev/poise-rs
git push -u origin main
scripts/protect-main.sh copyleftdev/poise-rs
```

The script creates or configures a public repository, enables issues,
discussions, and private vulnerability reporting, disables the wiki and
projects, configures merge hygiene, and installs the intended topics. After the
initial push, the protection script requires review, resolved conversations,
linear history, and the full CI/security check set. Review the settings
afterward.

Protect `main` with required pull requests and these CI jobs:

- Format, lint, and docs
- Documentation book
- Test / Rust stable
- Test / Rust 1.85.0
- Deterministic property laws
- Exhaustive scheduler models
- Package archives
- Licenses, advisories, bans, and sources
- RustSec advisory database

Allow GitHub Actions to create pull requests so Release-plz can maintain its
release PR. Keep environment approval on `crates-io` and
`crates-io-bootstrap`.

## First crates.io publication

Trusted publishing cannot create a crate name for its first release. Create a
short-lived crates.io token scoped to `publish-new` and `publish-update`, store
it as `CARGO_REGISTRY_TOKEN` only in the protected `crates-io-bootstrap`
environment, and run **Bootstrap crates.io publication** manually.

The workflow requires the exact confirmation text `publish-poise-0.1.0`, reruns
the gates, fully verifies the root `poise-core` archive, and delegates sequential
packaging, publication order, and tags to Release-plz. Dependent archives cannot
be verified against crates.io until their internal dependencies have completed
their first registry publication.

After all six crates exist:

1. Configure each crate's crates.io trusted publisher for this repository,
   `.github/workflows/release.yml`, and the `crates-io` environment.
2. Delete `CARGO_REGISTRY_TOKEN` from GitHub.
3. Set the repository variable `RELEASES_ENABLED=true`.
4. Run the regular **Release** workflow manually once and confirm it is a no-op.

Regular releases then use GitHub OIDC and no long-lived registry secret.

## Normal release flow

1. Merge Conventional Commits to `main` through green pull requests.
2. Release-plz updates or opens `chore: release Poise`.
3. Review SemVer decisions, API compatibility output, all manifest diffs,
   `Cargo.lock`, and `CHANGELOG.md`.
4. Merge the release PR without bypassing required checks.
5. Release-plz publishes unpublished workspace versions, creates per-crate tags,
   and creates GitHub releases.

All crates use the `poise` version group. A change that forces one dependent
crate to bump keeps the affected workspace versions coherent.

## Explicit bump hook

Automated release PRs are preferred. For an approved manual override:

```console
node scripts/bump-version.mjs patch
node scripts/bump-version.mjs minor
node scripts/bump-version.mjs 0.2.0
```

The command refuses a dirty tree unless `--allow-dirty` is explicit, changes the
workspace version and all internal registry requirements, refreshes
`Cargo.lock`, and runs Cargo plus the release-metadata check. The pre-commit hook
reruns the coherence check.

## Failure and recovery

Crates.io versions are immutable and cannot be overwritten. If a subset of the
workspace publishes, do not retag or retry an already published version. Fix the
cause, let Release-plz detect the unpublished packages, and resume. Yank only
when the published package is harmful; a yanked release remains downloadable by
existing lockfiles.

Never print, upload, or persist registry tokens in build artifacts or logs.
