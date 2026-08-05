/// The protocol-neutral result of one backend attempt.
///
/// Callers decide how transport and application errors map into these classes.
/// In particular, an application-level rejection is not necessarily a backend
/// failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Outcome {
    /// The backend successfully handled the attempt.
    Success,
    /// The backend failed the attempt.
    Failure,
    /// The backend explicitly indicated that it lacked capacity.
    Overloaded,
    /// The caller abandoned the attempt without a backend verdict.
    Cancelled,
}

impl Outcome {
    /// Returns whether the outcome is a backend-attributable failure.
    #[must_use]
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Failure | Self::Overloaded)
    }

    /// Returns whether the outcome should contribute a success-rate sample.
    ///
    /// Cancellation is excluded because it has no backend verdict.
    #[must_use]
    pub const fn is_observation(self) -> bool {
        !matches!(self, Self::Cancelled)
    }
}
