//! Readiness-correct Tower dispatch for Poise policies.
//!
//! [`Balance`] polls each eligible endpoint service, exposes only reserved
//! services to its selection policy, and consumes exactly one readiness
//! reservation per call. Its response future owns an RAII load guard, so
//! cancellation releases in-flight load without being mistaken for a completed
//! response.
//!
//! ```
//! use std::{convert::Infallible, future};
//! use poise_core::{Backend, policy::RoundRobin};
//! use poise_tower::{Balance, Endpoint};
//! use tower::{Service, service_fn};
//!
//! fn echo(request: u32) -> future::Ready<Result<u32, Infallible>> {
//!     future::ready(Ok(request))
//! }
//! let endpoints = vec![
//!     Endpoint::new(Backend::new("west"), service_fn(echo)),
//!     Endpoint::new(Backend::new("east"), service_fn(echo)),
//! ];
//! let mut balance = Balance::new(endpoints, RoundRobin::new());
//! # let mut context = std::task::Context::from_waker(std::task::Waker::noop());
//! assert!(balance.poll_ready(&mut context).is_ready());
//! let _response = balance.call(7);
//! ```
//!
//! # Support
//!
//! Support continued Poise engineering through
//! [TokenTip](https://tokentip.to/@copyleftdev).

#![forbid(unsafe_code)]

mod balance;
mod context;
#[cfg(feature = "discovery")]
mod discovery;
mod endpoint;
mod error;
mod future;
mod load;
mod observe;
#[cfg(feature = "discovery")]
mod streaming;

pub use balance::Balance;
pub use context::{NoContext, RequestContext, UseRequest};
#[cfg(feature = "discovery")]
pub use discovery::{
    DiscoveryBalance, DiscoveryInner, EndpointFactory, InFlightFactory, ReconcileError,
    ReconcileReport, in_flight_factory,
};
pub use endpoint::{Endpoint, Readiness};
pub use error::BalanceError;
pub use future::ResponseFuture;
pub use load::{LoadGuard, LoadTracker};
pub use observe::{IgnoreReadinessErrors, ObserveReadinessError};
#[cfg(feature = "discovery")]
pub use streaming::{
    StreamEndPolicy, StreamingConfig, StreamingDiscoveryBalance, StreamingError, StreamingParts,
    StreamingResponseFuture,
};
