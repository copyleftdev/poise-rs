use std::time::Duration;

use poise_core::{Outcome, PickError, Selection};

/// A fixed-cardinality policy-decision classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum DecisionKind {
    /// A candidate was selected.
    Selected,
    /// The candidate slice was empty.
    Empty,
    /// Candidates existed, but none was eligible.
    NoEligibleCandidates,
    /// Candidate weights could not be accumulated safely.
    WeightOverflow,
    /// Eligible candidates contained a duplicate stable identity.
    DuplicateIdentity,
    /// The policy's configured state capacity was exceeded.
    StateCapacityExceeded,
    /// A decision failure introduced by a newer core version.
    OtherFailure,
    /// Candidate load samples could not be accumulated safely.
    LoadOverflow,
    /// A priority rejected traffic in fail-closed panic mode.
    PanicRejected,
    /// Candidates reported conflicting topology metadata.
    InconsistentTopology,
}

impl DecisionKind {
    pub(crate) const COUNT: usize = 10;

    /// Every decision classification in stable declaration order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Selected,
        Self::Empty,
        Self::NoEligibleCandidates,
        Self::WeightOverflow,
        Self::DuplicateIdentity,
        Self::StateCapacityExceeded,
        Self::OtherFailure,
        Self::LoadOverflow,
        Self::PanicRejected,
        Self::InconsistentTopology,
    ];

    /// Returns the stable telemetry spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::Empty => "empty",
            Self::NoEligibleCandidates => "no_eligible_candidates",
            Self::WeightOverflow => "weight_overflow",
            Self::LoadOverflow => "load_overflow",
            Self::DuplicateIdentity => "duplicate_identity",
            Self::StateCapacityExceeded => "state_capacity_exceeded",
            Self::OtherFailure => "other_failure",
            Self::PanicRejected => "panic_rejected",
            Self::InconsistentTopology => "inconsistent_topology",
        }
    }

    pub(crate) const fn from_error(error: PickError) -> Self {
        match error {
            PickError::Empty => Self::Empty,
            PickError::NoEligibleCandidates => Self::NoEligibleCandidates,
            PickError::WeightOverflow => Self::WeightOverflow,
            PickError::LoadOverflow => Self::LoadOverflow,
            PickError::DuplicateIdentity => Self::DuplicateIdentity,
            PickError::StateCapacityExceeded => Self::StateCapacityExceeded,
            PickError::PanicRejected => Self::PanicRejected,
            PickError::InconsistentTopology => Self::InconsistentTopology,
            _ => Self::OtherFailure,
        }
    }
}

/// One policy decision, without backend identity or request labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecisionEvent {
    kind: DecisionKind,
    candidate_count: usize,
    selected_index: Option<usize>,
}

impl DecisionEvent {
    pub(crate) fn from_result(
        candidate_count: usize,
        result: &Result<Selection, PickError>,
    ) -> Self {
        match result {
            Ok(selection) => Self {
                kind: DecisionKind::Selected,
                candidate_count,
                selected_index: Some(selection.index()),
            },
            Err(error) => Self {
                kind: DecisionKind::from_error(*error),
                candidate_count,
                selected_index: None,
            },
        }
    }

    /// Creates an event for a fixed decision classification.
    ///
    /// `selected_index` is diagnostic context and is never used as a label by
    /// [`Metrics`](crate::Metrics).
    #[must_use]
    pub const fn new(
        kind: DecisionKind,
        candidate_count: usize,
        selected_index: Option<usize>,
    ) -> Self {
        Self {
            kind,
            candidate_count,
            selected_index,
        }
    }

    /// Returns the decision classification.
    #[must_use]
    pub const fn kind(self) -> DecisionKind {
        self.kind
    }

    /// Returns the size of the candidate slice supplied to the policy.
    #[must_use]
    pub const fn candidate_count(self) -> usize {
        self.candidate_count
    }

    /// Returns the selected slice index when selection succeeded.
    #[must_use]
    pub const fn selected_index(self) -> Option<usize> {
        self.selected_index
    }
}

#[cfg(test)]
mod decision_tests {
    use super::*;

    #[test]
    fn appended_failures_have_stable_classifications() {
        assert_eq!(DecisionKind::OtherFailure as usize, 6);
        assert_eq!(DecisionKind::LoadOverflow as usize, 7);
        assert_eq!(DecisionKind::PanicRejected as usize, 8);
        assert_eq!(DecisionKind::InconsistentTopology as usize, 9);
        assert_eq!(
            DecisionKind::from_error(PickError::LoadOverflow),
            DecisionKind::LoadOverflow
        );
        assert_eq!(DecisionKind::LoadOverflow.as_str(), "load_overflow");
        assert_eq!(
            DecisionKind::from_error(PickError::PanicRejected),
            DecisionKind::PanicRejected
        );
        assert_eq!(DecisionKind::PanicRejected.as_str(), "panic_rejected");
        assert_eq!(
            DecisionKind::from_error(PickError::InconsistentTopology),
            DecisionKind::InconsistentTopology
        );
    }
}

/// A fixed-cardinality backend-attempt classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum AttemptKind {
    /// The backend successfully handled the attempt.
    Success,
    /// The backend failed the attempt.
    Failure,
    /// The backend reported that it lacked capacity.
    Overloaded,
    /// The caller abandoned the attempt without a backend verdict.
    Cancelled,
    /// An outcome introduced by a newer core version.
    Other,
}

impl AttemptKind {
    pub(crate) const COUNT: usize = 5;

    /// Every attempt classification in stable declaration order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Success,
        Self::Failure,
        Self::Overloaded,
        Self::Cancelled,
        Self::Other,
    ];

    /// Returns the stable telemetry spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Overloaded => "overloaded",
            Self::Cancelled => "cancelled",
            Self::Other => "other",
        }
    }

    /// Classifies a protocol-neutral core outcome.
    #[must_use]
    pub const fn from_outcome(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Success => Self::Success,
            Outcome::Failure => Self::Failure,
            Outcome::Overloaded => Self::Overloaded,
            Outcome::Cancelled => Self::Cancelled,
            _ => Self::Other,
        }
    }
}

/// One completed or cancelled backend attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptEvent {
    kind: AttemptKind,
    elapsed: Duration,
}

impl AttemptEvent {
    /// Creates an attempt event.
    #[must_use]
    pub const fn new(kind: AttemptKind, elapsed: Duration) -> Self {
        Self { kind, elapsed }
    }

    /// Returns the attempt classification.
    #[must_use]
    pub const fn kind(self) -> AttemptKind {
        self.kind
    }

    /// Returns elapsed monotonic time for the attempt.
    #[must_use]
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

/// An isolated Tower readiness failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadinessFailure {
    endpoint_index: usize,
}

impl ReadinessFailure {
    /// Creates readiness-failure diagnostic context.
    #[must_use]
    pub const fn new(endpoint_index: usize) -> Self {
        Self { endpoint_index }
    }

    /// Returns the endpoint's current pool index.
    #[must_use]
    pub const fn endpoint_index(self) -> usize {
        self.endpoint_index
    }
}
