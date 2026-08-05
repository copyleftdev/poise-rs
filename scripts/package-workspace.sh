#!/usr/bin/env bash
set -euo pipefail

root_only=false
if [[ "${1:-}" == "--root-only" ]]; then
  root_only=true
elif [[ $# -gt 0 ]]; then
  echo "usage: scripts/package-workspace.sh [--root-only]" >&2
  exit 2
fi

# Explicit dependency order is documentation as well as execution policy.
packages=(
  poise-core
  poise-discovery
  poise-health
  poise-tower
  poise-tokio
  poise-observe
)

for package in "${packages[@]}"; do
  cargo package --locked --package "$package"
  if [[ "$root_only" == true ]]; then
    echo "bootstrap archive gate stops after poise-core; dependent crates require its first registry publication"
    break
  fi
done
