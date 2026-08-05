# Tokio integration

`poise-tokio` is an optional runtime boundary. Tokio does not appear in the
dependency graphs of `poise-core`, `poise-discovery`, `poise-health`, or the
base `poise-tower` adapter.

The crate's default features enable both integration families. With default
features disabled, `health` pulls in Tokio timing and `poise-health`, while
`discovery` pulls in only `poise-discovery`; the discovery wait futures are
executor-neutral and need no Tokio dependency of their own.

## Active health

`ActiveHealthRunner::run_once` waits until a probe is due, acquires the sole
reservation, and runs one caller-provided async probe. The probe translates its
protocol result into `ProbeResult`; the runner owns timing and state-machine
finalization.

```rust,no_run
use std::{num::NonZeroU32, time::Duration};

use poise_health::{ActiveHealth, ActiveHealthConfig, ProbeResult};
use poise_tokio::{ActiveHealthRunner, ProbeRunnerConfig};

# async fn example() {
let health = ActiveHealth::new(
    ActiveHealthConfig::new(
        Duration::from_secs(10),
        NonZeroU32::new(2).unwrap(),
        NonZeroU32::new(3).unwrap(),
    )
    .unwrap(),
);
let config = ProbeRunnerConfig::new(Duration::from_secs(2)).unwrap();
let mut runner = ActiveHealthRunner::new(
    health,
    || async {
        // Perform a protocol-specific request and classify its response here.
        ProbeResult::Healthy
    },
    config,
);

let report = runner.run_once().await.unwrap();
assert_eq!(report.result(), Some(ProbeResult::Healthy));
# }
```

A finite timeout is unhealthy by default. `ProbeTimeoutPolicy::Cancel` instead
preserves the existing health classification and threshold counters. Disabling
the timeout is explicit with `ProbeRunnerConfig::without_timeout()`.

The library deliberately does not spawn a background task or prescribe a
shutdown channel. Applications can loop over `run_once` and cancel it using
their existing task supervision or `tokio::select!`. Dropping `run_once` before
or during a probe cancels its reservation and schedules the next interval; it
never leaves the health state marked in flight.

If another owner already holds the active probe reservation, `run_once` returns
`ProbeRunnerError::ProbeAlreadyRunning` instead of spinning. The caller can
retry after that external owner completes or cancels its reservation.

The runner consistently converts `tokio::time::Instant` into the core's
clock-aware APIs. Consequently intervals, timeouts, and cancellation remain
deterministic under Tokio's paused test clock.

## Discovery waits

`next_snapshot` returns an allocation-free future borrowing a
`SnapshotStream`. It avoids requiring a general stream-extension dependency for
the common single-item wait. `wait_for_revision` creates a subscription before
examining its current state, closing the load-then-register race, and then waits
until the publisher reaches or passes the requested revision.

```rust,no_run
use poise_discovery::{Revision, Snapshot, snapshot_channel};
use poise_tokio::wait_for_revision;

# async fn example() {
let (mut publisher, reader) = snapshot_channel(Snapshot::empty());
publisher
    .publish(Snapshot::new(Revision::new(1), vec!["backend-a"]))
    .unwrap();

let snapshot = wait_for_revision(&reader, Revision::new(1))
    .await
    .expect("publisher remains open");
assert_eq!(snapshot.revision(), Revision::new(1));
# }
```

Snapshot streams coalesce bursts to the latest coherent state. They are state
notifications, not lossless event logs. A wait returns `None` when the sole
publisher closes before the requested revision is available.
