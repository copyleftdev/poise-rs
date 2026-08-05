use std::{
    cmp::Ordering,
    collections::VecDeque,
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroUsize},
};

#[cfg(loom)]
use loom::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex, MutexGuard};

use poise_core::{LoadMetric, Outcome};

/// Configuration for a rolling outcome window.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OutcomeWindowConfig {
    capacity: NonZeroUsize,
    minimum_samples: usize,
    failure_weight: NonZeroU32,
    overload_weight: NonZeroU32,
}

impl OutcomeWindowConfig {
    /// Creates a window that becomes active after its first sample.
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            minimum_samples: 1,
            failure_weight: NonZeroU32::MIN,
            overload_weight: match NonZeroU32::new(2) {
                Some(weight) => weight,
                None => unreachable!(),
            },
        }
    }

    /// Sets how many observations are required before a non-zero penalty is
    /// exposed.
    ///
    /// # Errors
    ///
    /// Returns [`OutcomeWindowConfigError`] if `minimum_samples` exceeds the
    /// window capacity.
    pub const fn with_minimum_samples(
        mut self,
        minimum_samples: usize,
    ) -> Result<Self, OutcomeWindowConfigError> {
        if minimum_samples > self.capacity.get() {
            return Err(OutcomeWindowConfigError::MinimumExceedsCapacity);
        }
        self.minimum_samples = minimum_samples;
        Ok(self)
    }

    /// Sets relative penalty units for failures and overload responses.
    #[must_use]
    pub const fn with_weights(
        mut self,
        failure_weight: NonZeroU32,
        overload_weight: NonZeroU32,
    ) -> Self {
        self.failure_weight = failure_weight;
        self.overload_weight = overload_weight;
        self
    }

    /// Returns the number of retained backend observations.
    #[must_use]
    pub const fn capacity(self) -> NonZeroUsize {
        self.capacity
    }

    /// Returns the activation sample count.
    #[must_use]
    pub const fn minimum_samples(self) -> usize {
        self.minimum_samples
    }

    /// Returns the failure penalty weight.
    #[must_use]
    pub const fn failure_weight(self) -> NonZeroU32 {
        self.failure_weight
    }

    /// Returns the overload penalty weight.
    #[must_use]
    pub const fn overload_weight(self) -> NonZeroU32 {
        self.overload_weight
    }
}

/// Invalid rolling-window configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum OutcomeWindowConfigError {
    /// The activation sample count is larger than the retained window.
    MinimumExceedsCapacity,
}

impl fmt::Display for OutcomeWindowConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MinimumExceedsCapacity => {
                f.write_str("minimum samples cannot exceed outcome window capacity")
            }
        }
    }
}

impl Error for OutcomeWindowConfigError {}

struct WindowState {
    outcomes: VecDeque<Outcome>,
    successes: usize,
    failures: usize,
    overloaded: usize,
    penalty_units: u128,
}

impl WindowState {
    fn new() -> Self {
        Self {
            outcomes: VecDeque::new(),
            successes: 0,
            failures: 0,
            overloaded: 0,
            penalty_units: 0,
        }
    }
}

struct WindowInner {
    config: OutcomeWindowConfig,
    state: Mutex<WindowState>,
}

/// A shared rolling window of backend-attributable outcomes.
///
/// Cancelled attempts are ignored. Overload and ordinary failure weights are
/// configurable, allowing an explicit capacity rejection to influence routing
/// more strongly than a generic failure.
#[derive(Clone)]
pub struct OutcomeWindow {
    inner: Arc<WindowInner>,
}

impl OutcomeWindow {
    /// Creates a window with failure weight `1` and overload weight `2`.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self::with_config(OutcomeWindowConfig::new(capacity))
    }

    /// Creates a rolling tracker from an explicit configuration.
    #[must_use]
    pub fn with_config(config: OutcomeWindowConfig) -> Self {
        Self {
            inner: Arc::new(WindowInner {
                config,
                state: Mutex::new(WindowState::new()),
            }),
        }
    }

    /// Records a backend outcome.
    ///
    /// [`Outcome::Cancelled`] is intentionally ignored.
    pub fn record(&self, outcome: Outcome) {
        if !matches!(
            outcome,
            Outcome::Success | Outcome::Failure | Outcome::Overloaded
        ) {
            return;
        }

        let mut state = self.lock_state();
        if state.outcomes.len() == self.inner.config.capacity.get() {
            if let Some(removed) = state.outcomes.pop_front() {
                self.remove_from_totals(&mut state, removed);
            }
        }
        self.add_to_totals(&mut state, outcome);
        state.outcomes.push_back(outcome);
    }

    /// Returns a coherent summary of the current window.
    #[must_use]
    pub fn stats(&self) -> OutcomeStats {
        let state = self.lock_state();
        let samples = state.outcomes.len();
        let penalty = if samples < self.inner.config.minimum_samples || samples == 0 {
            PenaltyScore::ZERO
        } else {
            #[allow(clippy::cast_precision_loss)]
            PenaltyScore::new(state.penalty_units as f64 / samples as f64)
        };

        OutcomeStats {
            successes: state.successes,
            failures: state.failures,
            overloaded: state.overloaded,
            penalty,
        }
    }

    /// Clears all retained observations.
    pub fn clear(&self) {
        *self.lock_state() = WindowState::new();
    }

    /// Returns the immutable configuration.
    #[must_use]
    pub fn config(&self) -> OutcomeWindowConfig {
        self.inner.config
    }

    fn add_to_totals(&self, state: &mut WindowState, outcome: Outcome) {
        match outcome {
            Outcome::Success => state.successes += 1,
            Outcome::Failure => {
                state.failures += 1;
                state.penalty_units += u128::from(self.inner.config.failure_weight.get());
            }
            Outcome::Overloaded => {
                state.overloaded += 1;
                state.penalty_units += u128::from(self.inner.config.overload_weight.get());
            }
            _ => {}
        }
    }

    fn remove_from_totals(&self, state: &mut WindowState, outcome: Outcome) {
        match outcome {
            Outcome::Success => state.successes -= 1,
            Outcome::Failure => {
                state.failures -= 1;
                state.penalty_units -= u128::from(self.inner.config.failure_weight.get());
            }
            Outcome::Overloaded => {
                state.overloaded -= 1;
                state.penalty_units -= u128::from(self.inner.config.overload_weight.get());
            }
            _ => {}
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, WindowState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for OutcomeWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutcomeWindow")
            .field("config", &self.config())
            .field("stats", &self.stats())
            .finish()
    }
}

impl LoadMetric for OutcomeWindow {
    type Metric = PenaltyScore;

    fn measure(&self) -> Self::Metric {
        self.stats().penalty()
    }
}

/// An immutable summary of a rolling outcome window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutcomeStats {
    successes: usize,
    failures: usize,
    overloaded: usize,
    penalty: PenaltyScore,
}

impl OutcomeStats {
    /// Returns the number of successful observations.
    #[must_use]
    pub const fn successes(self) -> usize {
        self.successes
    }

    /// Returns the number of ordinary failures.
    #[must_use]
    pub const fn failures(self) -> usize {
        self.failures
    }

    /// Returns the number of explicit overload responses.
    #[must_use]
    pub const fn overloaded(self) -> usize {
        self.overloaded
    }

    /// Returns the total number of backend observations.
    #[must_use]
    pub const fn samples(self) -> usize {
        self.successes + self.failures + self.overloaded
    }

    /// Returns the observed success ratio, or `None` for an empty window.
    #[must_use]
    pub fn success_rate(self) -> Option<f64> {
        let samples = self.samples();
        if samples == 0 {
            None
        } else {
            #[allow(clippy::cast_precision_loss)]
            Some(self.successes as f64 / samples as f64)
        }
    }

    /// Returns the average configured penalty per observation.
    #[must_use]
    pub const fn penalty(self) -> PenaltyScore {
        self.penalty
    }
}

/// A totally ordered average outcome penalty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenaltyScore(f64);

impl PenaltyScore {
    /// No observed penalty.
    pub const ZERO: Self = Self(0.0);

    fn new(value: f64) -> Self {
        debug_assert!(value.is_finite() && value >= 0.0);
        Self(value)
    }

    /// Returns average penalty units per recorded observation.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Eq for PenaltyScore {}

impl Ord for PenaltyScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for PenaltyScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, thread};

    use poise_core::{Backend, Policy, policy::LeastLoaded};

    use super::*;

    #[test]
    fn cancellations_are_not_backend_observations() {
        let window = OutcomeWindow::new(NonZeroUsize::new(4).unwrap());
        window.record(Outcome::Cancelled);

        let stats = window.stats();
        assert_eq!(stats.samples(), 0);
        assert_eq!(stats.success_rate(), None);
        assert_eq!(stats.penalty(), PenaltyScore::ZERO);
    }

    #[test]
    fn oldest_observations_are_evicted() {
        let window = OutcomeWindow::new(NonZeroUsize::new(2).unwrap());
        window.record(Outcome::Failure);
        window.record(Outcome::Success);
        window.record(Outcome::Success);

        let stats = window.stats();
        assert_eq!(stats.samples(), 2);
        assert_eq!(stats.successes(), 2);
        assert_eq!(stats.failures(), 0);
        assert_eq!(stats.penalty(), PenaltyScore::ZERO);
    }

    #[test]
    fn overload_has_its_configured_extra_weight() {
        let failure = OutcomeWindow::new(NonZeroUsize::new(4).unwrap());
        let overload = OutcomeWindow::new(NonZeroUsize::new(4).unwrap());
        failure.record(Outcome::Failure);
        overload.record(Outcome::Overloaded);

        assert!((failure.stats().penalty().get() - 1.0).abs() < f64::EPSILON);
        assert!((overload.stats().penalty().get() - 2.0).abs() < f64::EPSILON);
        assert!(overload.stats().penalty() > failure.stats().penalty());
    }

    #[test]
    fn minimum_samples_prevents_early_penalty() {
        let config = OutcomeWindowConfig::new(NonZeroUsize::new(4).unwrap())
            .with_minimum_samples(3)
            .unwrap();
        let window = OutcomeWindow::with_config(config);
        window.record(Outcome::Failure);
        window.record(Outcome::Failure);
        assert_eq!(window.stats().penalty(), PenaltyScore::ZERO);

        window.record(Outcome::Success);
        assert!(window.stats().penalty().get() > 0.0);
    }

    #[test]
    fn penalty_is_a_live_load_metric() {
        let unhealthy = OutcomeWindow::new(NonZeroUsize::new(8).unwrap());
        let healthy = OutcomeWindow::new(NonZeroUsize::new(8).unwrap());
        unhealthy.record(Outcome::Failure);
        healthy.record(Outcome::Success);
        let candidates = [
            Backend::new("unhealthy").with_load(unhealthy),
            Backend::new("healthy").with_load(healthy),
        ];

        let selected = LeastLoaded::new().pick(&candidates, &()).unwrap();
        assert_eq!(candidates[selected.index()].id(), &"healthy");
    }

    #[test]
    fn concurrent_recording_respects_window_capacity() {
        let capacity = NonZeroUsize::new(64).unwrap();
        let window = OutcomeWindow::new(capacity);
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let window = window.clone();
                thread::spawn(move || {
                    for sample in 0..1_000 {
                        let outcome = if (worker + sample) % 2 == 0 {
                            Outcome::Success
                        } else {
                            Outcome::Failure
                        };
                        window.record(outcome);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        let stats = window.stats();
        assert_eq!(stats.samples(), capacity.get());
        assert_eq!(stats.samples(), stats.successes() + stats.failures());
    }

    #[test]
    fn invalid_minimum_is_rejected() {
        let result =
            OutcomeWindowConfig::new(NonZeroUsize::new(2).unwrap()).with_minimum_samples(3);
        assert_eq!(
            result,
            Err(OutcomeWindowConfigError::MinimumExceedsCapacity)
        );
    }
}
