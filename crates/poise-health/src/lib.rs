//! Runtime-neutral backend health, circuit breaking, and outlier analysis.
//!
//! `poise-health` contains no protocol or async-runtime assumptions. Callers
//! classify attempts with [`Outcome`], acquire a [`CircuitPermit`] before
//! dispatch, and explicitly complete the permit when a backend verdict exists.
//! An implicitly dropped permit is a cancellation.
//!
//! Active checks use the same explicit-reservation model: the caller owns the
//! timer and transport, while [`ActiveHealth`] owns due-time coordination and
//! healthy/unhealthy thresholds. [`OutlierDetector`] separately analyzes a
//! group of rolling [`OutcomeStats`] without mutating backend state.
//!
//! ```
//! use std::{num::NonZeroU32, time::Duration};
//! use poise_health::{CircuitConfig, CircuitSnapshot, PassiveHealth};
//!
//! let config = CircuitConfig::new(NonZeroU32::new(2).unwrap(), Duration::from_secs(30))?;
//! let health = PassiveHealth::new(config);
//! health.try_acquire().unwrap().failure();
//! health.try_acquire().unwrap().overloaded();
//! assert!(matches!(health.snapshot(), CircuitSnapshot::Open { .. }));
//! # Ok::<(), poise_health::CircuitConfigError>(())
//! ```
//!
//! # Support
//!
//! Support continued Poise engineering through
//! [TokenTip](https://tokentip.to/@copyleftdev).

#![forbid(unsafe_code)]

mod active;
mod candidate;
mod circuit;
mod outcome_window;
mod outlier;

pub use active::{
    ActiveHealth, ActiveHealthConfig, ActiveHealthConfigError, ActiveProbe, ActiveSnapshot,
    ActiveStatus, ProbeRejected, ProbeResult, UnknownPolicy,
};
pub use candidate::{HealthChecked, HealthSignal};
pub use circuit::{
    CircuitConfig, CircuitConfigError, CircuitPermit, CircuitSnapshot, PassiveHealth, Rejected,
};
pub use outcome_window::{
    OutcomeStats, OutcomeWindow, OutcomeWindowConfig, OutcomeWindowConfigError, PenaltyScore,
};
pub use outlier::{OutlierConfig, OutlierConfigError, OutlierDetector, OutlierReport};
pub use poise_core::Outcome;
