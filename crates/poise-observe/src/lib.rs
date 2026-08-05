#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod attempt;
mod event;
mod metrics;
mod observer;
mod policy;
#[cfg(feature = "tower")]
mod tower;
#[cfg(feature = "tracing")]
mod tracing;

pub use attempt::Attempt;
pub use event::{AttemptEvent, AttemptKind, DecisionEvent, DecisionKind, ReadinessFailure};
pub use metrics::{
    ATTEMPT_LATENCY_BOUNDS, ATTEMPT_LATENCY_BUCKET_COUNT, LatencyBucket, LatencyHistogram, Metrics,
    MetricsSnapshot,
};
pub use observer::{Fanout, NoopObserver, Observer};
pub use policy::ObservedPolicy;
#[cfg(feature = "tower")]
pub use tower::TowerObserver;
#[cfg(feature = "tracing")]
pub use tracing::{TracedPolicy, TracingObserver};
