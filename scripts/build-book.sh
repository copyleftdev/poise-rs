#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

node --test scripts/check-book.test.mjs
node scripts/check-book.mjs
mdbook build
test -f site/book/index.html

echo "Built Poise engineering book at site/book/index.html"
