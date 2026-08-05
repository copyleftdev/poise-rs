#!/usr/bin/env bash
set -euo pipefail

runs="${POISE_FUZZ_RUNS:-1000}"
export ASAN_OPTIONS="${ASAN_OPTIONS:+${ASAN_OPTIONS}:}detect_leaks=0"

if ! command -v systemd-run >/dev/null 2>&1; then
  echo "fuzz smoke requires systemd-run for hard CPU and memory isolation" >&2
  exit 78
fi

for target in policy_state_machine topology_state_machine; do
  # Keep developer-machine smoke runs deliberately tiny. Full campaigns belong
  # on isolated CI workers with externally enforced resource limits.
  systemd-run --user --scope --quiet --collect \
    -p CPUQuota=25% \
    -p MemoryMax=192M \
    -p MemorySwapMax=0 \
    nice -n 15 timeout --signal=INT 30s \
    cargo +nightly fuzz run "$target" -- \
    "-runs=${runs}" \
    -max_len=256 \
    -rss_limit_mb=128 \
    -timeout=2 \
    -print_final_stats=1
done
