//! Runtime-independent load-balancing policy primitives.
//!
//! `poise-core` separates the description of a backend from the policy that
//! selects it. Policies inspect a borrowed candidate slice and return a
//! [`Selection`]; they never clone a backend or dispatch work.
//!
//! # Example
//!
//! ```
//! use poise_core::{Backend, Policy, policy::RoundRobin};
//!
//! let backends = [Backend::new("west"), Backend::new("east")];
//! let mut policy = RoundRobin::new();
//!
//! let first = policy.pick(&backends, &()).unwrap();
//! let second = policy.pick(&backends, &()).unwrap();
//! assert_eq!(backends[first.index()].id(), &"west");
//! assert_eq!(backends[second.index()].id(), &"east");
//! ```
//!
//! # Support
//!
//! Support continued Poise engineering through
//! [TokenTip](https://tokentip.to/@copyleftdev).

#![forbid(unsafe_code)]

mod backend;
mod error;
mod feedback;
mod hash;
mod load;
mod policy_trait;
mod probe;
mod selection;
mod weight;

pub mod policy;

pub use backend::{Backend, Candidate, Status};
pub use error::PickError;
pub use feedback::Outcome;
pub use hash::{Fnv1a64, FnvBuildHasher, mix64};
pub use load::{
    AtCapacity, InFlight, InFlightGuard, LoadMetric, LoadScore, PeakEwma, PeakEwmaConfigError,
    PeakEwmaGuard,
};
pub use policy_trait::{Policy, PolicyExt};
pub use probe::{
    ProbeDecisionError, ProbeEntry, ProbePool, ProbePoolConfig, ProbePoolConfigError, ProbeReading,
};
pub use selection::Selection;
pub use weight::{InvalidWeight, Weight};
