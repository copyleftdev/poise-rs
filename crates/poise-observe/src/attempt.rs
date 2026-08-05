use std::time::Instant;

use poise_core::Outcome;

use crate::{AttemptEvent, AttemptKind, Observer};

/// An RAII observation for one backend attempt.
///
/// Explicit completion records the caller's classified outcome. Dropping an
/// unfinished attempt records cancellation, so early returns and task
/// cancellation cannot silently disappear from telemetry.
#[derive(Debug)]
pub struct Attempt<O: Observer> {
    observer: O,
    started: Instant,
    finished: bool,
}

impl<O: Observer> Attempt<O> {
    /// Starts an attempt using the process monotonic clock.
    #[must_use]
    pub fn new(observer: O) -> Self {
        Self {
            observer,
            started: Instant::now(),
            finished: false,
        }
    }

    /// Returns the observer.
    #[must_use]
    pub fn observer(&self) -> &O {
        &self.observer
    }

    /// Completes the attempt with a protocol-neutral outcome.
    pub fn complete(mut self, outcome: Outcome) {
        self.finish(AttemptKind::from_outcome(outcome));
    }

    /// Completes a successful attempt.
    pub fn success(self) {
        self.complete(Outcome::Success);
    }

    /// Completes a failed attempt.
    pub fn failure(self) {
        self.complete(Outcome::Failure);
    }

    /// Completes an overloaded attempt.
    pub fn overloaded(self) {
        self.complete(Outcome::Overloaded);
    }

    /// Explicitly cancels the attempt.
    pub fn cancel(self) {
        self.complete(Outcome::Cancelled);
    }

    fn finish(&mut self, kind: AttemptKind) {
        if !self.finished {
            self.observer
                .observe_attempt(AttemptEvent::new(kind, self.started.elapsed()));
            self.finished = true;
        }
    }
}

impl<O: Observer> Drop for Attempt<O> {
    fn drop(&mut self) {
        self.finish(AttemptKind::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use crate::{AttemptKind, Metrics};

    use super::*;

    #[test]
    fn completion_and_drop_are_classified_exactly_once() {
        let metrics = Metrics::new();
        Attempt::new(metrics.clone()).success();
        drop(Attempt::new(metrics.clone()));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.attempts(AttemptKind::Success), 1);
        assert_eq!(snapshot.attempts(AttemptKind::Cancelled), 1);
        assert_eq!(snapshot.attempt_latency().count(), 2);
    }
}
