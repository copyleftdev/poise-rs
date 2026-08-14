# shellcheck shell=bash
#
# Shared resource cage for the expensive verification scripts. Source it; do not
# execute it.
#
# Mutation campaigns and Loom model checking are the two jobs in this repository
# capable of overwhelming the machine they run on. An unbounded mutation campaign
# has already frozen a workstation hard enough to corrupt the git object store.
#
# Isolation is preferred but cannot be required. These scripts run both on
# developer workstations and on GitHub-hosted runners, and `systemd-run --user`
# needs a user session bus that a CI runner generally does not have. Testing for
# the binary is not enough: `systemd-run` exists on those runners and then fails
# at invocation. So probe by actually creating a throwaway scope, and degrade to
# the remaining bounds when that fails -- a CI worker is already an externally
# limited sandbox, which is the environment the workstation cage is imitating.
#
# Set POISE_REQUIRE_ISOLATION=1 to turn a missing cage into a hard failure.

# Returns success when a transient user scope can actually be created.
bounds_isolation_available() {
  command -v systemd-run >/dev/null 2>&1 || return 1
  systemd-run --user --scope --quiet --collect \
    -p MemoryMax=64M true >/dev/null 2>&1
}

# bounds_run <cpu_quota> <memory_max> <timeout_seconds> <label> -- <command...>
#
# Runs the command under a memory and CPU cage when one is available, niced and
# under a wall-clock backstop either way. Returns the command's exit status.
bounds_run() {
  local cpu_quota="$1" memory_max="$2" timeout_seconds="$3" label="$4"
  shift 4
  [[ ${1-} == "--" ]] && shift

  local prefix=()
  if bounds_isolation_available; then
    prefix=(
      systemd-run --user --scope --quiet --collect
      -p CPUQuota="$cpu_quota"
      -p MemoryMax="$memory_max"
      -p MemorySwapMax=0
    )
    echo "${label} bounds: ${memory_max} memory, no swap, ${cpu_quota} CPU," \
      "${timeout_seconds}s wall clock"
  elif [[ ${POISE_REQUIRE_ISOLATION:-0} == 1 ]]; then
    echo "${label} requires systemd-run for hard CPU and memory isolation" >&2
    return 78
  else
    # Worth saying out loud. The parallelism caps and the wall clock still
    # apply; the memory ceiling does not, so nothing here stops a runaway from
    # exhausting an unbounded host.
    echo "${label}: no usable systemd scope, running without a memory ceiling;" \
      "parallelism caps and the ${timeout_seconds}s wall clock still apply" >&2
  fi

  "${prefix[@]}" nice -n 15 timeout --signal=INT "$timeout_seconds" "$@"
}

# Explains the stopping condition when a bounded run is cut short, so a limit
# that has genuinely become too tight stays visible as a limit rather than being
# reported as a verification failure. Returns success when it handled the status.
bounds_explain_stop() {
  local status="$1" memory_max="$2" timeout_seconds="$3" raise_hint="$4"

  if (( status == 124 || status == 130 )); then
    echo "stopped after exceeding the ${timeout_seconds}s wall clock" >&2
    echo "raise ${raise_hint%%:*} for a legitimately longer run" >&2
    return 0
  fi
  # The cgroup terminates the scope rather than letting it swap, which surfaces
  # as SIGTERM or SIGKILL depending on how far the run got.
  if (( status == 137 || status == 143 )); then
    echo "stopped at the ${memory_max} memory ceiling" >&2
    echo "raise ${raise_hint##*:} only after confirming the growth is expected" >&2
    return 0
  fi
  return 1
}
