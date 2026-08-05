//! Selection policy implementations.

mod bounded_load;
mod least_loaded;
mod locality;
mod maglev;
mod power_of_two;
mod priority;
mod random;
mod rendezvous;
mod ring_hash;
mod round_robin;
mod smooth_weighted_round_robin;
mod weighted_random;
mod weighted_rendezvous;

pub use bounded_load::{
    BoundedLoadConfig, BoundedLoadConfigError, BoundedLoadDecision, BoundedLoadRendezvous,
    DEFAULT_BALANCE_FACTOR_PERCENT,
};
pub use least_loaded::LeastLoaded;
pub use locality::{LocalityCandidate, LocalityDecision, LocalityWeightedRandom, Localized};
pub use maglev::{
    DEFAULT_TABLE_SIZE, MAX_TABLE_SIZE, Maglev, MaglevConfig, MaglevConfigError, MaglevUpdate,
};
pub use power_of_two::PowerOfTwoChoices;
pub use priority::{
    DEFAULT_OVERPROVISIONING_FACTOR_PERCENT, DEFAULT_PANIC_THRESHOLD_PERCENT, PanicMode,
    Prioritized, PriorityCandidate, PriorityConfig, PriorityConfigError, PriorityDecision,
    PriorityMode, PriorityWeightedRandom,
};
pub use random::Random;
pub use rendezvous::Rendezvous;
pub use ring_hash::{RingHash, RingHashConfig, RingHashConfigError, RingUpdate};
pub use round_robin::RoundRobin;
pub use smooth_weighted_round_robin::SmoothWeightedRoundRobin;
pub use weighted_random::WeightedRandom;
pub use weighted_rendezvous::WeightedRendezvous;

use crate::PickError;

fn no_candidate_error(candidate_count: usize) -> PickError {
    if candidate_count == 0 {
        PickError::Empty
    } else {
        PickError::NoEligibleCandidates
    }
}
