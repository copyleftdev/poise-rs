#!/usr/bin/env bash
set -euo pipefail

repository="${1:-}"
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "usage: scripts/bootstrap-github.sh OWNER/REPOSITORY" >&2
  exit 2
fi

gh auth status
git rev-parse --is-inside-work-tree >/dev/null
owner="${repository%%/*}"
name="${repository##*/}"
homepage="https://${owner}.github.io/${name}/"

if ! gh repo view "$repository" >/dev/null 2>&1; then
  gh repo create "$repository" \
    --public \
    --source=. \
    --remote=origin \
    --description="Composable, rigorously verified load-balancing primitives for Rust"
fi

gh repo edit "$repository" \
  --visibility public \
  --accept-visibility-change-consequences \
  --enable-issues \
  --enable-discussions \
  --enable-projects=false \
  --enable-wiki=false \
  --enable-auto-merge \
  --enable-merge-commit=false \
  --enable-rebase-merge \
  --enable-squash-merge \
  --squash-merge-commit-message=pr-title-description \
  --allow-update-branch \
  --delete-branch-on-merge \
  --homepage="$homepage" \
  --description="Composable, rigorously verified load-balancing primitives for Rust"

topics=(
  rust
  load-balancing
  distributed-systems
  service-discovery
  consistent-hashing
  circuit-breaker
  tower
  tokio
  observability
  concurrency
  networking
  infrastructure
)
for topic in "${topics[@]}"; do
  gh repo edit "$repository" --add-topic "$topic"
done

gh label create release --repo "$repository" --color dbe51d --description "Dependency-aware release preparation" --force
gh label create needs-triage --repo "$repository" --color cfd3d7 --description "Needs initial maintainer assessment" --force
gh label create needs-design --repo "$repository" --color 6f42c1 --description "Requires contract and architecture discussion" --force
gh label create proposal --repo "$repository" --color 1d76db --description "Proposed feature or behavioral contract" --force

gh api --method PUT "repos/$repository/private-vulnerability-reporting" >/dev/null

gh api --method PUT "repos/$repository/actions/permissions/workflow" +  -f default_workflow_permissions=read +  -F can_approve_pull_request_reviews=true >/dev/null

for environment in crates-io crates-io-bootstrap; do
  gh api --method PUT "repos/$repository/environments/$environment" >/dev/null
done

if gh api "repos/$repository/pages" >/dev/null 2>&1; then
  gh api --method PUT "repos/$repository/pages" -f build_type=workflow >/dev/null
else
  gh api --method POST "repos/$repository/pages" -f build_type=workflow >/dev/null
fi

echo "Configured public metadata, security reporting, Actions, environments, and Pages for $repository"
echo "Push main, run scripts/protect-main.sh $repository, then follow docs/releasing.md."
