#!/usr/bin/env bash
set -euo pipefail

# Property tests are randomized during ordinary development. Mutation results
# must be reproducible so a survivor cannot disappear merely due to a new seed.
export PROPTEST_RNG_SEED="${PROPTEST_RNG_SEED:-1592614637}"

# A mutation campaign is the most resource-hostile thing this repository runs:
# it rebuilds and retests the crate once per mutant, 705 times. Every bound
# below exists because an unbounded campaign froze a 64-core workstation hard
# enough to corrupt the git object store. The limits are enforced here rather
# than documented as flags a caller has to remember.
# Run from the workspace root so cargo-mutants resolves the workspace and writes
# mutants.out where the gate below looks for it, whatever the caller's directory.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
scratch_root="${POISE_MUTANTS_SCRATCH:-${repo_root}/target/mutants-scratch}"
jobs="${POISE_MUTANTS_JOBS:-1}"
max_jobs="${POISE_MUTANTS_MAX_JOBS:-4}"
build_jobs="${POISE_MUTANTS_BUILD_JOBS:-8}"
memory_per_job_mb="${POISE_MUTANTS_MEMORY_PER_JOB_MB:-4096}"
campaign_timeout="${POISE_MUTANTS_TIMEOUT:-5400}"

if ! command -v systemd-run >/dev/null 2>&1; then
  echo "mutation campaigns require systemd-run for hard CPU and memory isolation" >&2
  exit 78
fi

# Bound 1: nested parallelism. cargo-mutants runs `jobs` cargo invocations and
# each one spawns its own `CARGO_BUILD_JOBS` rustc processes, so the real
# process count is the product. Left to their defaults on a many-core machine
# that product is nproc^2 -- this is the multiplication that actually melts the
# box, and neither factor bounds the other.
if [[ ! $jobs =~ ^[1-9][0-9]*$ ]]; then
  echo "POISE_MUTANTS_JOBS must be a positive integer, got '${jobs}'" >&2
  exit 64
fi
if (( jobs > max_jobs )); then
  echo "clamping ${jobs} mutation workers to the ${max_jobs}-worker ceiling" >&2
  jobs=$max_jobs
fi
export CARGO_BUILD_JOBS="$build_jobs"

# Bound 2: scratch location. cargo-mutants copies the source tree and the whole
# target directory into a temp dir per worker -- 555M each here -- and it takes
# that directory from TMPDIR. On this machine /tmp is a 124G tmpfs, so the
# default put every copy in RAM and pushed roughly 390G of copy traffic through
# unreclaimable pages over a full campaign. Scratch must live on real storage.
mkdir -p "$scratch_root"
scratch_fs=$(stat -f -c %T "$scratch_root")
case "$scratch_fs" in
  tmpfs | ramfs)
    echo "mutation scratch directory ${scratch_root} is on ${scratch_fs}, which is RAM-backed;" >&2
    echo "set POISE_MUTANTS_SCRATCH to a path on real storage" >&2
    exit 78
    ;;
esac
export TMPDIR="$scratch_root"

# Worker scratch directories leak if a campaign is killed rather than exiting,
# and each one is over half a gigabyte.
# shellcheck disable=SC2317  # reached through the EXIT trap
cleanup() { rm -rf "${scratch_root:?}"/cargo-mutants-*.tmp; }
trap cleanup EXIT

# A caller may still pass --jobs; the documented invocation does. Take the value
# out of the argument list, put it through the same ceiling, and pass the
# clamped result on. Leaving the caller's flag in place would let it win over
# anything set here, since an explicit flag outranks CARGO_MUTANTS_JOBS.
forwarded=()
while (( $# > 0 )); do
  case "$1" in
    -j | --jobs)
      requested="${2-}"
      shift 2 || shift
      ;;
    -j=* | --jobs=*)
      requested="${1#*=}"
      shift
      ;;
    -j[0-9]*)
      requested="${1#-j}"
      shift
      ;;
    # Forwarded, not consumed: the gate below has to read the report cargo-mutants
    # actually wrote, or it would judge the run against a stale mutants.out.
    -o | --output)
      output_root="${2-}"
      forwarded+=("$1")
      if (( $# >= 2 )); then
        forwarded+=("$2")
        shift
      fi
      shift
      ;;
    -o=* | --output=*)
      output_root="${1#*=}"
      forwarded+=("$1")
      shift
      ;;
    *)
      forwarded+=("$1")
      shift
      ;;
  esac
done
if [[ -n ${requested-} ]]; then
  if [[ ! $requested =~ ^[1-9][0-9]*$ ]]; then
    echo "--jobs must be a positive integer, got '${requested}'" >&2
    exit 64
  fi
  if (( requested > max_jobs )); then
    echo "clamping the requested ${requested} mutation workers to the ${max_jobs}-worker ceiling" >&2
  fi
  jobs=$(( requested > max_jobs ? max_jobs : requested ))
fi

# Bound 3: a hard ceiling the campaign cannot talk its way out of. Measured peak
# is 1.4G per worker, so the allowance is generous; MemorySwapMax=0 is the
# load-bearing half. Without it the kernel answers memory pressure by thrashing
# swap, which is what makes the whole desktop unresponsive instead of just
# killing the campaign. Failing loudly beats taking the machine down.
memory_max="$(( jobs * memory_per_job_mb ))M"
cpu_quota="$(( jobs * build_jobs * 100 ))%"

echo "mutation campaign bounds: ${jobs} worker(s) x ${build_jobs} build jobs," \
  "${memory_max} memory, no swap, ${cpu_quota} CPU, ${campaign_timeout}s wall clock," \
  "scratch on ${scratch_fs} at ${scratch_root}"

set +e
systemd-run --user --scope --quiet --collect \
  -p CPUQuota="$cpu_quota" \
  -p MemoryMax="$memory_max" \
  -p MemorySwapMax=0 \
  nice -n 15 timeout --signal=INT "$campaign_timeout" \
  cargo mutants \
  --package poise-core \
  --all-features \
  --jobs "$jobs" \
  ${forwarded[@]+"${forwarded[@]}"}
status=$?
set -e

# Distinguish "the gate failed" from "the bounds stopped us", so a limit that is
# genuinely too tight for a growing crate is visible as itself rather than
# reported as a mutation failure.
if (( status == 124 || status == 130 )); then
  echo "mutation campaign exceeded its ${campaign_timeout}s wall clock and was stopped" >&2
  echo "raise POISE_MUTANTS_TIMEOUT for a legitimately longer campaign" >&2
  exit "$status"
fi
# The cgroup terminates the scope rather than letting it swap, which surfaces as
# SIGTERM or SIGKILL depending on how far the campaign got before it was stopped.
if (( status == 137 || status == 143 )); then
  echo "mutation campaign was stopped at the ${memory_max} memory ceiling" >&2
  echo "raise POISE_MUTANTS_MEMORY_PER_JOB_MB only after confirming the growth is expected" >&2
  exit "$status"
fi

# cargo-mutants reports timeout-only campaigns as exit 3. A mutant that turns a
# sub-second suite into a 20-second hang has been detected, but we still reject
# the run if any terminating mutant survived. Baseline failures and every other
# cargo-mutants error retain their original status.
missed="${output_root:-${CARGO_MUTANTS_OUTPUT:-.}}/mutants.out/missed.txt"
if [[ $status -eq 3 && -f $missed && ! -s $missed ]]; then
  echo "mutation gate: all terminating mutants caught; timeout mutants detected by the watchdog"
  exit 0
fi

exit "$status"
