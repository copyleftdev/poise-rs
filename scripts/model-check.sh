#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
# shellcheck source=scripts/lib/resource-bounds.sh
source "${repo_root}/scripts/lib/resource-bounds.sh"

export RUSTFLAGS="${RUSTFLAGS:-} --cfg loom"

build_jobs="${POISE_LOOM_BUILD_JOBS:-8}"
test_threads="${POISE_LOOM_TEST_THREADS:-4}"
memory_max="${POISE_LOOM_MEMORY_MAX:-4096M}"
run_timeout="${POISE_LOOM_TIMEOUT:-1800}"

# Loom explores a model's interleavings exhaustively, so its cost is
# combinatorial in the number of threads and shared operations a model contains.
# Today's models are small -- all of them complete within 100 branches against
# loom's default ceiling of 1000 -- but the failure mode of adding one more
# concurrent step is a state-space explosion rather than a gradual slowdown.
#
# Two multipliers bound that. `CARGO_BUILD_JOBS` caps rustc parallelism during
# the separate --cfg loom build, and `RUST_TEST_THREADS` caps how many models
# explore at once: the harness otherwise runs one test per core, so peak memory
# is per-model cost times core count rather than per-model cost.
export CARGO_BUILD_JOBS="$build_jobs"
export RUST_TEST_THREADS="$test_threads"

# Make loom's own ceiling explicit and reviewable rather than implicit. Exceeding
# it aborts the model with a loud failure, so this bound cannot silently shrink
# coverage -- a model that outgrows it fails until someone raises it deliberately.
#
# Actively cleared, not merely left unset: `loom::model` builds through
# `Builder::new`, which reads `LOOM_MAX_PREEMPTIONS` from the environment and
# applies it as a preemption bound. An inherited value would silently prune
# interleavings -- cheaper exploration by making it partial, without saying so --
# which is precisely the property this gate exists to establish. Leaving the
# variable unset here does nothing about one exported by the caller.
unset LOOM_MAX_PREEMPTIONS
export LOOM_MAX_BRANCHES="${LOOM_MAX_BRANCHES:-1000}"

cpu_quota="$(( build_jobs * 100 ))%"

set +e
bounds_run "$cpu_quota" "$memory_max" "$run_timeout" "loom model check" -- \
  bash -c '
    set -euo pipefail
    cargo test --release -p poise-core --test loom_in_flight
    cargo test --release -p poise-core --test loom_probe_pool
    cargo test --release -p poise-health --test loom_health
  '
status=$?
set -e

if (( status != 0 )); then
  bounds_explain_stop "$status" "$memory_max" "$run_timeout" \
    "POISE_LOOM_TIMEOUT:POISE_LOOM_MEMORY_MAX" || true
fi

exit "$status"
