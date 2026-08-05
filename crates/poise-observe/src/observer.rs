use std::sync::Arc;

use crate::{AttemptEvent, DecisionEvent, ReadinessFailure};

/// Receives bounded observability records from Poise adapters.
///
/// Methods take shared references so one observer can be cloned or shared by
/// concurrent balancers. Default methods discard records, allowing observers
/// to implement only the signals they need.
pub trait Observer {
    /// Observes one selection-policy result.
    fn observe_decision(&self, _event: DecisionEvent) {}

    /// Observes one completed or cancelled backend attempt.
    fn observe_attempt(&self, _event: AttemptEvent) {}

    /// Observes one endpoint readiness failure.
    fn observe_readiness_failure(&self, _event: ReadinessFailure) {}
}

impl<O> Observer for &O
where
    O: Observer + ?Sized,
{
    fn observe_decision(&self, event: DecisionEvent) {
        O::observe_decision(self, event);
    }

    fn observe_attempt(&self, event: AttemptEvent) {
        O::observe_attempt(self, event);
    }

    fn observe_readiness_failure(&self, event: ReadinessFailure) {
        O::observe_readiness_failure(self, event);
    }
}

impl<O> Observer for Arc<O>
where
    O: Observer + ?Sized,
{
    fn observe_decision(&self, event: DecisionEvent) {
        O::observe_decision(self, event);
    }

    fn observe_attempt(&self, event: AttemptEvent) {
        O::observe_attempt(self, event);
    }

    fn observe_readiness_failure(&self, event: ReadinessFailure) {
        O::observe_readiness_failure(self, event);
    }
}

/// Discards every observability record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct NoopObserver;

impl Observer for NoopObserver {}

/// Sends each record to two observers in deterministic order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Fanout<A, B> {
    first: A,
    second: B,
}

impl<A, B> Fanout<A, B> {
    /// Creates an observer that calls `first` and then `second`.
    #[must_use]
    pub const fn new(first: A, second: B) -> Self {
        Self { first, second }
    }

    /// Returns the first observer.
    #[must_use]
    pub const fn first(&self) -> &A {
        &self.first
    }

    /// Returns the second observer.
    #[must_use]
    pub const fn second(&self) -> &B {
        &self.second
    }

    /// Decomposes the fanout.
    #[must_use]
    pub fn into_parts(self) -> (A, B) {
        (self.first, self.second)
    }
}

impl<A, B> Observer for Fanout<A, B>
where
    A: Observer,
    B: Observer,
{
    fn observe_decision(&self, event: DecisionEvent) {
        self.first.observe_decision(event);
        self.second.observe_decision(event);
    }

    fn observe_attempt(&self, event: AttemptEvent) {
        self.first.observe_attempt(event);
        self.second.observe_attempt(event);
    }

    fn observe_readiness_failure(&self, event: ReadinessFailure) {
        self.first.observe_readiness_failure(event);
        self.second.observe_readiness_failure(event);
    }
}

#[cfg(test)]
mod tests {
    use crate::{DecisionEvent, DecisionKind, Metrics};

    use super::{Fanout, Observer};

    #[test]
    fn fanout_delivers_to_both_observers() {
        let first = Metrics::new();
        let second = Metrics::new();
        let observer = Fanout::new(first.clone(), second.clone());

        observer.observe_decision(DecisionEvent::new(DecisionKind::Selected, 1, Some(0)));

        assert_eq!(first.snapshot().decisions(DecisionKind::Selected), 1);
        assert_eq!(second.snapshot().decisions(DecisionKind::Selected), 1);
    }
}
