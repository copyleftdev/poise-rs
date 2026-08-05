use std::{error::Error, fmt};

use poise_core::{AtCapacity, PickError};

/// A Tower balancing or endpoint failure.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BalanceError<E> {
    /// The balancer contains no endpoints.
    NoEndpoints,
    /// Endpoints exist, but their candidate metadata excludes all of them.
    NoEligibleEndpoints,
    /// Eligible endpoints exist, but every corresponding service has failed
    /// readiness.
    NoReadyEndpoints,
    /// The policy could not select from the ready endpoint set.
    Selection(PickError),
    /// A custom policy returned an index outside the endpoint slice.
    InvalidSelection {
        /// Returned endpoint index.
        index: usize,
        /// Endpoint count at selection time.
        len: usize,
    },
    /// The selected endpoint's load tracker rejected a new attempt.
    AtCapacity {
        /// Selected endpoint index.
        index: usize,
        /// Capacity failure from the load tracker.
        source: AtCapacity,
    },
    /// An endpoint failed readiness or returned a response error.
    Endpoint {
        /// Endpoint index in the balancer.
        index: usize,
        /// Error returned by the endpoint service.
        source: E,
    },
}

impl<E> BalanceError<E> {
    /// Returns the associated endpoint index, when one exists.
    #[must_use]
    pub const fn endpoint_index(&self) -> Option<usize> {
        match self {
            Self::InvalidSelection { index, .. }
            | Self::AtCapacity { index, .. }
            | Self::Endpoint { index, .. } => Some(*index),
            Self::NoEndpoints
            | Self::NoEligibleEndpoints
            | Self::NoReadyEndpoints
            | Self::Selection(_) => None,
        }
    }
}

impl<E: fmt::Display> fmt::Display for BalanceError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEndpoints => f.write_str("the Tower balancer has no endpoints"),
            Self::NoEligibleEndpoints => {
                f.write_str("no endpoint metadata currently permits dispatch")
            }
            Self::NoReadyEndpoints => {
                f.write_str("every eligible endpoint service has failed readiness")
            }
            Self::Selection(error) => write!(f, "the selection policy failed: {error}"),
            Self::InvalidSelection { index, len } => {
                write!(
                    f,
                    "the policy selected endpoint {index}, but the pool length is {len}"
                )
            }
            Self::AtCapacity { index, source } => {
                write!(f, "endpoint {index} rejected dispatch capacity: {source}")
            }
            Self::Endpoint { index, source } => {
                write!(f, "endpoint {index} failed: {source}")
            }
        }
    }
}

impl<E> Error for BalanceError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Selection(error) => Some(error),
            Self::AtCapacity { source, .. } => Some(source),
            Self::Endpoint { source, .. } => Some(source),
            Self::NoEndpoints
            | Self::NoEligibleEndpoints
            | Self::NoReadyEndpoints
            | Self::InvalidSelection { .. } => None,
        }
    }
}
