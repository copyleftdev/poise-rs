#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

node --test scripts/check-book.test.mjs
node scripts/check-book.mjs
mdbook build
test -f site/book/index.html

# The control-loop diagram is committed under the book source, which is the one
# copy the book, the README, and GitHub's file view can all resolve. The landing
# page is served from site/, so it needs the same bytes there; copying here
# rather than committing a second copy keeps the two from drifting apart.
mkdir -p site/assets
cp docs/assets/control-loop.svg site/assets/control-loop.svg

echo "Built Poise engineering book at site/book/index.html"
