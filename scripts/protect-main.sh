#!/usr/bin/env bash
set -euo pipefail

repository="${1:-}"
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "usage: scripts/protect-main.sh OWNER/REPOSITORY" >&2
  exit 2
fi

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
    "required_approving_review_count": 1,
    "require_last_push_approval": true
  },
  "restrictions": null,
  "required_conversation_resolution": true,
  "required_linear_history": true,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "block_creations": false
}
JSON

echo "Protected $repository main with review, conversation, history, and CI requirements"
