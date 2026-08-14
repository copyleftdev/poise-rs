#!/usr/bin/env bash
set -euo pipefail

repository="${1:-}"
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "usage: scripts/protect-main.sh OWNER/REPOSITORY" >&2
  exit 2
fi

# This repository has a single maintainer with write access. A required
# approving review cannot be satisfied under that arrangement: GitHub refuses to
# let an author approve their own pull request, and it only counts approvals
# from collaborators who have write access, so a read-only second account does
# not help either. Requiring one approval therefore did not mean "reviewed" --
# it meant "unmergeable without an administrative bypass", and a guardrail
# routinely bypassed teaches everyone to bypass guardrails.
#
# So the human approval is not required, and every machine-enforced gate is
# kept: pull requests are still required, all nine checks must pass, admins are
# not exempt, history stays linear, conversations must be resolved, and force
# pushes and deletions stay banned. Raise the count again the moment a second
# collaborator has write access.
gh auth status
gh api --method PUT "repos/$repository/branches/main/protection" --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "Format, lint, and docs",
      "Documentation book",
      "Test / Rust stable",
      "Test / Rust 1.85.0",
      "Deterministic property laws",
      "Exhaustive scheduler models",
      "Package archives",
      "Licenses, advisories, bans, and sources",
      "RustSec advisory database"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 0,
    "require_last_push_approval": false
  },
  "restrictions": null,
  "required_conversation_resolution": true,
  "required_linear_history": true,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "block_creations": false
}
JSON

echo "Protected $repository main with CI, conversation, and history requirements;" \
  "pull requests required, approvals not (single maintainer with write access)"
