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
#
# The probe carries every property the real invocation sets, not a
# representative one. Property support varies by systemd version, so probing a
# narrower set would report a cage as available and then fail at the point of
# use -- the same mistake as testing for the `systemd-run` binary and assuming
# the scope would build.
bounds_isolation_available() {
  command -v systemd-run >/dev/null 2>&1 || return 1
  systemd-run --user --scope --quiet --collect \
    -p CPUQuota=100% \
    -p MemoryMax=64M \
    -p MemorySwapMax=0 \
    -p RuntimeMaxSec=30 \
    true >/dev/null 2>&1
}

# bounds_run <cpu_quota> <memory_max> <timeout_seconds> <label> -- <command...>
#
# Runs the command under a memory and CPU cage when one is available, niced and
# under a wall-clock backstop either way. Returns the command's exit status.
bounds_run() {
  local cpu_quota="$1" memory_max="$2" timeout_seconds="$3" label="$4"
  shift 4
  [[ ${1-} == "--" ]] && shift

  # SIGINT first so cargo and its children can unwind and report, then SIGKILL
  # after a grace period, because a wedged process is exactly what a backstop is
  # for and INT is ignorable. `timeout` only signals its direct child, though,
  # so a grandchild that outlives its parent escapes it entirely.
  local kill_grace="${POISE_BOUNDS_KILL_GRACE:-30}"
  # The scope closes that gap where one exists: systemd stops the whole cgroup,
  # descendants included, and does not depend on any process honoring a signal.
  # It is deliberately later than the `timeout` deadline so the graceful path
  # runs first and only a genuinely stuck run reaches this.
  local scope_deadline=$(( timeout_seconds + kill_grace + 15 ))

  # Whether a memory ceiling actually exists, which the degraded path below does
  # not create. bounds_explain_stop must not name a ceiling that was never set.
  BOUNDS_CAGED=0

  local prefix=()
  if bounds_isolation_available; then
    BOUNDS_CAGED=1
    prefix=(
      systemd-run --user --scope --quiet --collect
      -p CPUQuota="$cpu_quota"
      -p MemoryMax="$memory_max"
      -p MemorySwapMax=0
      -p RuntimeMaxSec="$scope_deadline"
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

  local started=$SECONDS status=0
  "${prefix[@]}" nice -n 15 \
    timeout --signal=INT --kill-after="$kill_grace" "$timeout_seconds" "$@" || status=$?
  # Exit status alone cannot separate a forced timeout kill from a memory kill:
  # GNU timeout reports 137 after escalating, and so does the cgroup. Elapsed
  # time can, so record it for bounds_explain_stop.
  BOUNDS_ELAPSED=$(( SECONDS - started ))
  return "$status"
}

# Explains the stopping condition when a bounded run is cut short, so a limit
# that has genuinely become too tight stays visible as a limit rather than being
# reported as a verification failure. Returns success when it handled the status.
bounds_explain_stop() {
  local status="$1" memory_max="$2" timeout_seconds="$3" raise_hint="$4"
  local elapsed="${BOUNDS_ELAPSED:-0}" caged="${BOUNDS_CAGED:-0}"

  # No status proves the deadline fired. 124 is timeout's own convention but it
  # also passes a command's exit status through unchanged, so a tool exiting 124
  # on its own would be misread; the signal statuses are shared with the cgroup.
  # Only reaching the deadline distinguishes them, and `SECONDS` truncates, so
  # allow a second of slack rather than missing the boundary case.
  local reached_deadline=0
  (( elapsed + 1 >= timeout_seconds )) && reached_deadline=1

  if (( reached_deadline && (status == 124 || status == 130 || status == 137 || status == 143) )); then
    echo "stopped after exceeding the ${timeout_seconds}s wall clock" >&2
    echo "raise ${raise_hint%%:*} for a legitimately longer run" >&2
    return 0
  fi
  # Short of the deadline, a signal means the cgroup terminated the scope rather
  # than letting it swap -- but only if there was a cgroup. Without one there is
  # no ceiling to name, and the status belongs to the command.
  if (( caged && (status == 137 || status == 143) )); then
    echo "stopped at the ${memory_max} memory ceiling after ${elapsed}s" >&2
    echo "raise ${raise_hint##*:} only after confirming the growth is expected" >&2
    return 0
  fi
  return 1
}
