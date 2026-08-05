#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "health")]
mod health;
#[cfg(feature = "discovery")]
mod snapshot;

#[cfg(feature = "health")]
pub use health::{
    ActiveHealthRunner, ProbeReport, ProbeRunnerConfig, ProbeRunnerConfigError, ProbeRunnerError,
    ProbeTimeoutPolicy, TokioProbe,
};
#[cfg(feature = "discovery")]
pub use snapshot::{NextSnapshot, next_snapshot, wait_for_revision};
