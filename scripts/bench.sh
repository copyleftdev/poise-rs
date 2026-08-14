#!/usr/bin/env bash
set -euo pipefail

# Run the criterion benchmarks, optionally saving or comparing a baseline.
#
#   scripts/bench.sh                          # measure and report
#   scripts/bench.sh --save-baseline main     # record a baseline
#   scripts/bench.sh --baseline main          # compare against one
#
# Deliberately NOT wrapped in the resource cage that `mutants-core.sh` and
# `model-check.sh` use. Those bound a job whose cost is the problem; here the
# measurement is the product, and a CPU quota, swap ban, or `nice` level would
# change the thing being measured. Throttled numbers are not conservative
# numbers, they are wrong ones. A benchmark is bounded by running it on an idle
# machine, not by capping it.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Comparable stochastic runs, matching the seeds the benches use internally.
export PROPTEST_RNG_SEED="${PROPTEST_RNG_SEED:-1592614637}"

# A baseline is only meaningful against a quiet machine. This is advice rather
# than enforcement: refusing to run would be worse than reporting a caveat, and
# only the operator knows whether the load is theirs.
if [[ -r /proc/loadavg ]]; then
  load=$(cut -d' ' -f1 /proc/loadavg)
  cores=$(nproc)
  # Integer comparison in tenths, since load is fractional and this is bash.
  load_tenths=${load/./}
  load_tenths=$((10#${load_tenths#0}))
  if (( load_tenths > cores * 10 / 4 )); then
    echo "warning: load average ${load} on ${cores} cores; measurements will be noisy" >&2
    echo "warning: record baselines on an otherwise idle machine" >&2
  fi
fi

echo "benchmarking $(rustc --version)"
echo "on $(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ //') with $(nproc) cores"

# `--benches` restricts this to the criterion targets. Integration test binaries
# are also bench targets by default, and they parse arguments with libtest,
# which rejects criterion's flags.
cargo bench --locked -p poise-core --benches -- "$@"
