#!/usr/bin/env bash
set -euo pipefail

# Property tests are randomized during ordinary development. Mutation results
# must be reproducible so a survivor cannot disappear merely due to a new seed.
export PROPTEST_RNG_SEED="${PROPTEST_RNG_SEED:-1592614637}"

set +e
cargo mutants \
  --package poise-core \
  --all-features \
  "$@"
status=$?
set -e

# cargo-mutants reports timeout-only campaigns as exit 3. A mutant that turns a
# sub-second suite into a 20-second hang has been detected, but we still reject
# the run if any terminating mutant survived. Baseline failures and every other
# cargo-mutants error retain their original status.
if [[ $status -eq 3 && -f mutants.out/missed.txt && ! -s mutants.out/missed.txt ]]; then
  echo "mutation gate: all terminating mutants caught; timeout mutants detected by the watchdog"
  exit 0
fi

exit "$status"
