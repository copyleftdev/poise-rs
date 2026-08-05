use poise_core::{PickError, Policy, Selection};

use crate::{DecisionEvent, Observer};

/// A policy decorator that reports each decision without changing its result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservedPolicy<P, O> {
    policy: P,
    observer: O,
}

impl<P, O> ObservedPolicy<P, O> {
    /// Wraps a policy with an observer.
    #[must_use]
    pub const fn new(policy: P, observer: O) -> Self {
        Self { policy, observer }
    }

    /// Returns the underlying policy.
    #[must_use]
    pub const fn policy(&self) -> &P {
        &self.policy
    }

    /// Returns mutable access to the underlying policy.
    #[must_use]
    pub const fn policy_mut(&mut self) -> &mut P {
        &mut self.policy
    }

    /// Returns the observer.
    #[must_use]
    pub const fn observer(&self) -> &O {
        &self.observer
    }

    /// Decomposes the decorator.
    #[must_use]
    pub fn into_parts(self) -> (P, O) {
        (self.policy, self.observer)
    }
}

impl<P, O, C, Context> Policy<C, Context> for ObservedPolicy<P, O>
where
    P: Policy<C, Context>,
    O: Observer,
    Context: ?Sized,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        let result = self.policy.pick(candidates, context);
        self.observer
            .observe_decision(DecisionEvent::from_result(candidates.len(), &result));
        result
    }
}

#[cfg(test)]
mod tests {
    use poise_core::{Backend, PickError, Policy, policy::RoundRobin};

    use crate::{DecisionKind, Metrics};

    use super::ObservedPolicy;

    #[test]
    fn decorator_preserves_policy_results_and_state() {
        let candidates = [Backend::new("a"), Backend::new("b")];
        let metrics = Metrics::new();
        let mut policy = ObservedPolicy::new(RoundRobin::new(), metrics.clone());

        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 0);
        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 1);
        let empty: [Backend<&str>; 0] = [];
        assert_eq!(policy.pick(&empty, &()), Err(PickError::Empty));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.decisions(DecisionKind::Selected), 2);
        assert_eq!(snapshot.decisions(DecisionKind::Empty), 1);
    }
}
