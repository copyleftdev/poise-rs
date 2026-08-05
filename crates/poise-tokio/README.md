# poise-tokio

Tokio integration for Poise's runtime-neutral load-balancing primitives.

The crate deliberately owns no balancing or health state. It supplies:

- cancellation-safe scheduling and timeout policy for `poise_health::ActiveHealth`;
- allocation-free async waits over `poise_discovery::SnapshotStream`.

This keeps Tokio out of the core, discovery, health, and Tower dependency
surfaces while providing the common runtime plumbing applications otherwise
have to reproduce.

The default feature set contains both integrations. Disable default features
and select `health` or `discovery` when only one boundary is needed.

```rust
use std::{num::NonZeroU32, time::Duration};

use poise_health::{ActiveHealth, ActiveHealthConfig, ProbeResult};
use poise_tokio::{ActiveHealthRunner, ProbeRunnerConfig};

let health = ActiveHealth::new(
    ActiveHealthConfig::new(
        Duration::from_secs(10),
        NonZeroU32::new(2).unwrap(),
        NonZeroU32::new(3).unwrap(),
    )
    .unwrap(),
);
let runner = ActiveHealthRunner::new(
    health,
    || async { ProbeResult::Healthy },
    ProbeRunnerConfig::new(Duration::from_secs(2)).unwrap(),
);

assert_eq!(runner.config().timeout(), Some(Duration::from_secs(2)));
```
