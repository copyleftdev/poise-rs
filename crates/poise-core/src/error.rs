use std::{error::Error, fmt};

/// Why a policy could not select a backend.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PickError {
    /// The supplied candidate set was empty.
    Empty,
    /// Candidates were supplied, but none accepted new work.
    NoEligibleCandidates,
    /// Summing candidate weights exceeded the supported range.
    WeightOverflow,
    /// Two eligible candidates advertised the same stable identity.
    DuplicateIdentity,
    /// A policy's configured state or table limit cannot represent the
    /// candidate set.
    StateCapacityExceeded,
    /// Summing candidate load samples or the prospective request exceeded the
    /// supported range.
    LoadOverflow,
    /// A selected priority entered fail-closed panic mode.
    PanicRejected,
    /// Candidates reported conflicting metadata for one topology group.
    InconsistentTopology,
}

impl fmt::Display for PickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the candidate set is empty"),
            Self::NoEligibleCandidates => {
                f.write_str("no candidate is eligible to receive new work")
            }
            Self::WeightOverflow => f.write_str("the total candidate weight overflowed u64"),
            Self::DuplicateIdentity => {
                f.write_str("eligible candidates must have unique stable identities")
            }
            Self::StateCapacityExceeded => {
                f.write_str("the candidate set exceeds the policy state capacity")
            }
            Self::LoadOverflow => f.write_str("the total candidate load overflowed u64"),
            Self::PanicRejected => f.write_str("the selected priority rejected traffic in panic"),
            Self::InconsistentTopology => {
                f.write_str("candidates reported inconsistent topology metadata")
            }
        }
    }
}

impl Error for PickError {}
