# Release engineering

Poise uses one version across six crates and publishes them in dependency order.
Normal releases are pull-request driven; direct `cargo publish` from a maintainer
laptop is an emergency procedure, not the default.

## Current release state

The six crate names were claimed with version `0.1.0` on August 5, 2026, after
the repository's CI, package, Loom, property, and mutation gates passed. New
versions remain gated on all of the following:

1. GitHub Actions, protected environments, branch protection, and private
   vulnerability reporting remain enabled. Branch protection requires a pull
   request and all nine checks, and exempts nobody; it does not require an
   approving review, because a single maintainer with write access cannot
   supply one. Item 3 is therefore a maintainer obligation rather than a
   mechanically enforced gate, and should be read as one.
2. The complete CI, MSRV, Loom, package, and mutation gates are green on the
   exact release commit.
3. The release PR's SemVer decisions, archive contents, changelog, and API
   compatibility report receive maintainer review.
4. Each package publishes from the protected `crates-io` environment through a
   crates.io trusted publisher.

`node scripts/check-release-metadata.mjs --publishable` enforces the mechanical
portion of this list.

## Cargo metadata

The shared package identity is defined once:

```toml
[workspace.package]
version = "0.1.1"
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
release PR. Keep environment approval on `crates-io`; the bootstrap environment
is retired after its token is deleted.

## Completed crates.io bootstrap

Trusted publishing could not create the crate names for their first releases.
The initial `0.1.0` versions were therefore published in dependency order with
a short-lived crates.io token scoped to `publish-new` and `publish-update`.

The bootstrap-only GitHub workflow is not a normal release mechanism and must
not be rerun. Dependent archives could not be verified against crates.io until
their internal dependencies completed their first registry publication, so the
bootstrap proceeded from `poise-core` through the dependency graph and verified
each exact registry version before continuing.

After bootstrap, maintainers must complete this one-time handoff:

1. Configure each crate's crates.io trusted publisher for this repository,
   `.github/workflows/release.yml`, and the `crates-io` environment.
2. Delete `CARGO_REGISTRY_TOKEN` from GitHub.
3. Delete or lock the retired `crates-io-bootstrap` environment.
4. Set the repository variable `RELEASES_ENABLED=true`.
5. Run the regular **Release** workflow manually once and confirm it is a no-op.

Regular releases then use GitHub OIDC and no long-lived registry secret.

## Normal release flow

1. Merge Conventional Commits to `main` through green pull requests.
2. Release-plz updates or opens `chore: release Poise`.
3. Review SemVer decisions, API compatibility output, all manifest diffs,
   `Cargo.lock`, and `CHANGELOG.md`.
4. Merge the release PR without bypassing required checks.
5. Release-plz publishes unpublished workspace versions, creates one workspace
   tag, and creates one GitHub release.

All crates use the `poise` version group. A change that forces one dependent
crate to bump keeps the affected workspace versions coherent.

Because every crate shares one version, the workspace publishes a single
`v0.1.1`-style tag rather than six package-qualified ones. Release-plz has no
notion of tagging a version group, so `git_tag_enable` and `git_release_enable`
are disabled workspace-wide and re-enabled only on `poise-core`, which every
other crate depends on. Enabling either flag for a second package would make
that package contend for the same tag name.

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
