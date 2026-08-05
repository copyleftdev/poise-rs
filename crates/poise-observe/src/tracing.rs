use poise_core::{PickError, Policy, Selection};
use tracing::{Level, debug_span, event, field};

use crate::{AttemptEvent, DecisionEvent, Observer, ReadinessFailure};

/// Emits structured Poise events through the `tracing` ecosystem.
///
/// Events contain fixed classifications and numeric diagnostic context. They
/// never contain backend identity, request keys, or error strings.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TracingObserver;

impl Observer for TracingObserver {
    fn observe_decision(&self, event: DecisionEvent) {
        event!(
            target: "poise::decision",
            Level::DEBUG,
            kind = event.kind().as_str(),
            candidate_count = ?event.candidate_count(),
            selected_index = ?event.selected_index(),
            "load-balancing policy decision"
        );
    }

    fn observe_attempt(&self, event: AttemptEvent) {
        event!(
            target: "poise::attempt",
            Level::DEBUG,
            kind = event.kind().as_str(),
            elapsed_micros = ?event.elapsed().as_micros(),
            "backend attempt finished"
        );
    }

    fn observe_readiness_failure(&self, event: ReadinessFailure) {
        event!(
            target: "poise::readiness",
            Level::WARN,
            endpoint_index = ?event.endpoint_index(),
            "endpoint readiness failed"
        );
    }
}

/// A policy decorator that creates one structured span around each `pick`.
///
/// Unlike [`ObservedPolicy`](crate::ObservedPolicy), this wrapper records the
/// policy call's synchronous duration through the tracing subscriber. The span
/// deliberately excludes backend and request identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TracedPolicy<P> {
    policy: P,
}

impl<P> TracedPolicy<P> {
    /// Wraps a policy in bounded tracing spans.
    #[must_use]
    pub const fn new(policy: P) -> Self {
        Self { policy }
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

    /// Consumes the decorator and returns the underlying policy.
    #[must_use]
    pub fn into_inner(self) -> P {
        self.policy
    }
}

impl<P, C, Context> Policy<C, Context> for TracedPolicy<P>
where
    P: Policy<C, Context>,
    Context: ?Sized,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        let span = debug_span!(
            target: "poise::decision",
            "poise.policy.pick",
            candidate_count = ?candidates.len(),
            kind = field::Empty,
            selected_index = field::Empty,
        );
        let _entered = span.enter();
        let result = self.policy.pick(candidates, context);
        let event = DecisionEvent::from_result(candidates.len(), &result);
        span.record("kind", event.kind().as_str());
        if let Some(index) = event.selected_index() {
            span.record("selected_index", field::debug(index));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use poise_core::{Backend, Policy, policy::RoundRobin};
    use tracing::{
        Event, Metadata, Subscriber,
        span::{Attributes, Id, Record},
        subscriber::with_default,
    };

    use crate::{
        AttemptEvent, AttemptKind, DecisionEvent, DecisionKind, Observer, ReadinessFailure,
    };

    use super::{TracedPolicy, TracingObserver};

    #[derive(Clone)]
    struct CountingSubscriber {
        spans: Arc<AtomicU64>,
        events: Arc<AtomicU64>,
    }

    impl Subscriber for CountingSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _attributes: &Attributes<'_>) -> Id {
            Id::from_u64(self.spans.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, _event: &Event<'_>) {
            self.events.fetch_add(1, Ordering::Relaxed);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[test]
    fn traced_policy_preserves_selection_without_a_subscriber() {
        let candidates = [Backend::new("a"), Backend::new("b")];
        let mut policy = TracedPolicy::new(RoundRobin::new());

        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 0);
        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 1);
    }

    #[test]
    fn enabled_subscriber_receives_span_and_structured_events() {
        let spans = Arc::new(AtomicU64::new(0));
        let events = Arc::new(AtomicU64::new(0));
        let subscriber = CountingSubscriber {
            spans: Arc::clone(&spans),
            events: Arc::clone(&events),
        };

        with_default(subscriber, || {
            let mut policy = TracedPolicy::new(RoundRobin::new());
            policy.pick(&[Backend::new("a")], &()).unwrap();

            let observer = TracingObserver;
            observer.observe_decision(DecisionEvent::new(DecisionKind::Selected, 1, Some(0)));
            observer.observe_attempt(AttemptEvent::new(
                AttemptKind::Success,
                std::time::Duration::from_millis(1),
            ));
            observer.observe_readiness_failure(ReadinessFailure::new(0));
        });

        assert_eq!(spans.load(Ordering::Relaxed), 1);
        assert_eq!(events.load(Ordering::Relaxed), 3);
    }
}
