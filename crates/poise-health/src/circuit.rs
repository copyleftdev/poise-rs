use std::{
    error::Error,
    fmt,
    num::NonZeroU32,
    time::{Duration, Instant},
};

#[cfg(loom)]
use loom::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex, MutexGuard};

use poise_core::Outcome;

/// Passive circuit-breaker configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CircuitConfig {
    failure_threshold: NonZeroU32,
    success_threshold: NonZeroU32,
    open_for: Duration,
    half_open_max_in_flight: NonZeroU32,
}

impl CircuitConfig {
    /// Creates a circuit requiring one successful half-open probe and allowing
    /// one probe at a time.
    ///
    /// # Errors
    ///
    /// Returns [`CircuitConfigError`] when the open duration is zero.
    pub fn new(
        failure_threshold: NonZeroU32,
        open_for: Duration,
    ) -> Result<Self, CircuitConfigError> {
        if open_for.is_zero() {
            return Err(CircuitConfigError::ZeroOpenDuration);
        }
        Ok(Self {
            failure_threshold,
            success_threshold: NonZeroU32::MIN,
            open_for,
            half_open_max_in_flight: NonZeroU32::MIN,
        })
    }

    /// Sets the successful probes required to close a half-open circuit.
    #[must_use]
    pub const fn with_success_threshold(mut self, threshold: NonZeroU32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Sets the maximum simultaneous half-open probes.
    #[must_use]
    pub const fn with_half_open_max_in_flight(mut self, limit: NonZeroU32) -> Self {
        self.half_open_max_in_flight = limit;
        self
    }

    /// Returns the consecutive failures required to open the circuit.
    #[must_use]
    pub const fn failure_threshold(self) -> NonZeroU32 {
        self.failure_threshold
    }

    /// Returns the successful half-open probes required to close the circuit.
    #[must_use]
    pub const fn success_threshold(self) -> NonZeroU32 {
        self.success_threshold
    }

    /// Returns how long an open circuit rejects attempts.
    #[must_use]
    pub const fn open_for(self) -> Duration {
        self.open_for
    }

    /// Returns the half-open probe concurrency limit.
    #[must_use]
    pub const fn half_open_max_in_flight(self) -> NonZeroU32 {
        self.half_open_max_in_flight
    }
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self::new(
            NonZeroU32::new(5).expect("five is non-zero"),
            Duration::from_secs(30),
        )
        .expect("the default circuit duration is non-zero")
    }
}

/// Invalid circuit configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CircuitConfigError {
    /// An open duration must allow a real rejection interval.
    ZeroOpenDuration,
}

impl fmt::Display for CircuitConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroOpenDuration => f.write_str("circuit open duration must be non-zero"),
        }
    }
}

impl Error for CircuitConfigError {}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Closed { consecutive_failures: u32 },
    Open { opened_at: Instant },
    HalfOpen { in_flight: u32, successes: u32 },
}

struct CircuitInner {
    config: CircuitConfig,
    mode: Mutex<Mode>,
}

/// A shared passive circuit breaker.
///
/// Every dispatched attempt should hold a [`CircuitPermit`]. A permit completed
/// with failure or overload contributes to failure accrual. Dropped permits are
/// cancellations and do not affect health.
#[derive(Clone)]
pub struct PassiveHealth {
    inner: Arc<CircuitInner>,
}

impl PassiveHealth {
    /// Creates a closed passive circuit.
    #[must_use]
    pub fn new(config: CircuitConfig) -> Self {
        Self {
            inner: Arc::new(CircuitInner {
                config,
                mode: Mutex::new(Mode::Closed {
                    consecutive_failures: 0,
                }),
            }),
        }
    }

    /// Attempts to reserve permission for one backend attempt.
    ///
    /// Closed circuits permit normal attempts. Once the cooldown elapses, an
    /// open circuit becomes half-open and admits only the configured number of
    /// probes.
    ///
    /// # Errors
    ///
    /// Returns [`Rejected`] while the circuit is open or all half-open probe
    /// slots are occupied.
    pub fn try_acquire(&self) -> Result<CircuitPermit, Rejected> {
        let now = Instant::now();
        let mut mode = self.lock_mode();
        self.refresh(&mut mode, now);

        let kind = match &mut *mode {
            Mode::Closed { .. } => PermitKind::Closed,
            Mode::Open { opened_at } => {
                return Err(Rejected::Open {
                    retry_after: self.retry_after(*opened_at, now),
                });
            }
            Mode::HalfOpen { in_flight, .. } => {
                let limit = self.inner.config.half_open_max_in_flight;
                if *in_flight >= limit.get() {
                    return Err(Rejected::ProbeLimit { limit });
                }
                *in_flight += 1;
                PermitKind::Probe
            }
        };

        Ok(CircuitPermit {
            health: self.clone(),
            kind,
            finished: false,
        })
    }

    /// Returns whether an attempt could currently acquire a permit.
    ///
    /// This is advisory; callers must still handle a race at [`try_acquire`](Self::try_acquire).
    #[must_use]
    pub fn is_available(&self) -> bool {
        let now = Instant::now();
        let mut mode = self.lock_mode();
        self.refresh(&mut mode, now);
        match *mode {
            Mode::Closed { .. } => true,
            Mode::Open { .. } => false,
            Mode::HalfOpen { in_flight, .. } => {
                in_flight < self.inner.config.half_open_max_in_flight.get()
            }
        }
    }

    /// Returns a coherent state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> CircuitSnapshot {
        let now = Instant::now();
        let mut mode = self.lock_mode();
        self.refresh(&mut mode, now);
        match *mode {
            Mode::Closed {
                consecutive_failures,
            } => CircuitSnapshot::Closed {
                consecutive_failures,
            },
            Mode::Open { opened_at } => CircuitSnapshot::Open {
                retry_after: self.retry_after(opened_at, now),
            },
            Mode::HalfOpen {
                in_flight,
                successes,
            } => CircuitSnapshot::HalfOpen {
                in_flight,
                successes,
                max_in_flight: self.inner.config.half_open_max_in_flight,
            },
        }
    }

    /// Immediately opens the circuit and starts a fresh cooldown.
    pub fn force_open(&self) {
        let now = Instant::now();
        *self.lock_mode() = Mode::Open { opened_at: now };
    }

    /// Immediately closes the circuit and clears failure accrual.
    pub fn force_close(&self) {
        *self.lock_mode() = Mode::Closed {
            consecutive_failures: 0,
        };
    }

    /// Returns the immutable configuration.
    #[must_use]
    pub fn config(&self) -> CircuitConfig {
        self.inner.config
    }

    fn finish(&self, kind: PermitKind, outcome: Outcome) {
        let now = Instant::now();
        let mut mode = self.lock_mode();
        match (kind, &mut *mode) {
            (
                PermitKind::Closed,
                Mode::Closed {
                    consecutive_failures,
                },
            ) => match outcome {
                Outcome::Success => *consecutive_failures = 0,
                Outcome::Failure | Outcome::Overloaded => {
                    *consecutive_failures = consecutive_failures.saturating_add(1);
                    if *consecutive_failures >= self.inner.config.failure_threshold.get() {
                        *mode = Mode::Open { opened_at: now };
                    }
                }
                _ => {}
            },
            (
                PermitKind::Probe,
                Mode::HalfOpen {
                    in_flight,
                    successes,
                },
            ) => {
                *in_flight = in_flight.saturating_sub(1);
                match outcome {
                    Outcome::Success => {
                        *successes = successes.saturating_add(1);
                        if *successes >= self.inner.config.success_threshold.get() {
                            *mode = Mode::Closed {
                                consecutive_failures: 0,
                            };
                        }
                    }
                    Outcome::Failure | Outcome::Overloaded => {
                        *mode = Mode::Open { opened_at: now };
                    }
                    _ => {}
                }
            }
            // A result from an older circuit epoch cannot mutate the new one.
            _ => {}
        }
    }

    fn refresh(&self, mode: &mut Mode, now: Instant) {
        if matches!(
            *mode,
            Mode::Open { opened_at }
                if now.saturating_duration_since(opened_at) >= self.inner.config.open_for
        ) {
            *mode = Mode::HalfOpen {
                in_flight: 0,
                successes: 0,
            };
        }
    }

    fn retry_after(&self, opened_at: Instant, now: Instant) -> Duration {
        self.inner
            .config
            .open_for
            .saturating_sub(now.saturating_duration_since(opened_at))
    }

    fn lock_mode(&self) -> MutexGuard<'_, Mode> {
        self.inner
            .mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for PassiveHealth {
    fn default() -> Self {
        Self::new(CircuitConfig::default())
    }
}

impl fmt::Debug for PassiveHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassiveHealth")
            .field("config", &self.config())
            .field("state", &self.snapshot())
            .finish()
    }
}

/// An immutable passive-circuit state snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CircuitSnapshot {
    /// Attempts flow normally.
    Closed {
        /// Consecutive backend-attributable failures.
        consecutive_failures: u32,
    },
    /// Attempts are rejected for the remaining cooldown.
    Open {
        /// Time remaining before half-open probing.
        retry_after: Duration,
    },
    /// A bounded number of recovery probes may run.
    HalfOpen {
        /// Probes currently outstanding.
        in_flight: u32,
        /// Successful probes in the current half-open epoch.
        successes: u32,
        /// Maximum simultaneous probes.
        max_in_flight: NonZeroU32,
    },
}

/// Why a circuit rejected an attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Rejected {
    /// The cooldown has not elapsed.
    Open {
        /// Time remaining before a probe may be admitted.
        retry_after: Duration,
    },
    /// Every half-open probe slot is occupied.
    ProbeLimit {
        /// Configured simultaneous probe limit.
        limit: NonZeroU32,
    },
}

impl Rejected {
    /// Returns the known retry delay for an open circuit.
    #[must_use]
    pub const fn retry_after(self) -> Option<Duration> {
        match self {
            Self::Open { retry_after } => Some(retry_after),
            Self::ProbeLimit { .. } => None,
        }
    }
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { retry_after } => {
                write!(f, "circuit is open for another {retry_after:?}")
            }
            Self::ProbeLimit { limit } => {
                write!(f, "all {limit} half-open probe slots are occupied")
            }
        }
    }
}

impl Error for Rejected {}

#[derive(Clone, Copy, Debug)]
enum PermitKind {
    Closed,
    Probe,
}

/// An RAII attempt permission from a [`PassiveHealth`] circuit.
pub struct CircuitPermit {
    health: PassiveHealth,
    kind: PermitKind,
    finished: bool,
}

impl CircuitPermit {
    /// Completes the attempt with a classified backend outcome.
    pub fn complete(mut self, outcome: Outcome) {
        self.finish(outcome);
    }

    /// Completes a successful attempt.
    pub fn success(self) {
        self.complete(Outcome::Success);
    }

    /// Completes a failed attempt.
    pub fn failure(self) {
        self.complete(Outcome::Failure);
    }

    /// Completes an explicitly overloaded attempt.
    pub fn overloaded(self) {
        self.complete(Outcome::Overloaded);
    }

    /// Cancels the attempt without changing passive health.
    pub fn cancel(self) {
        self.complete(Outcome::Cancelled);
    }

    fn finish(&mut self, outcome: Outcome) {
        if self.finished {
            return;
        }
        self.health.finish(self.kind, outcome);
        self.finished = true;
    }
}

impl fmt::Debug for CircuitPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitPermit")
            .field("kind", &self.kind)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl Drop for CircuitPermit {
    fn drop(&mut self) {
        self.finish(Outcome::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU32,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU32, Ordering},
        },
        thread,
    };

    use super::*;

    fn config(failures: u32, open_for: Duration) -> CircuitConfig {
        CircuitConfig::new(NonZeroU32::new(failures).unwrap(), open_for).unwrap()
    }

    #[test]
    fn consecutive_failures_open_the_circuit() {
        let health = PassiveHealth::new(config(2, Duration::from_secs(1)));
        health.try_acquire().unwrap().failure();
        assert_eq!(
            health.snapshot(),
            CircuitSnapshot::Closed {
                consecutive_failures: 1
            }
        );

        health.try_acquire().unwrap().overloaded();
        assert!(matches!(health.snapshot(), CircuitSnapshot::Open { .. }));
        assert!(!health.is_available());
        assert!(matches!(health.try_acquire(), Err(Rejected::Open { .. })));
    }

    #[test]
    fn success_and_cancellation_reset_or_preserve_failure_accrual() {
        let health = PassiveHealth::new(config(2, Duration::from_secs(1)));
        health.try_acquire().unwrap().failure();
        drop(health.try_acquire().unwrap());
        assert_eq!(
            health.snapshot(),
            CircuitSnapshot::Closed {
                consecutive_failures: 1
            }
        );

        health.try_acquire().unwrap().success();
        assert_eq!(
            health.snapshot(),
            CircuitSnapshot::Closed {
                consecutive_failures: 0
            }
        );
    }

    #[test]
    fn cooldown_admits_only_the_configured_probe_count() {
        let health = PassiveHealth::new(config(1, Duration::from_millis(2)));
        health.try_acquire().unwrap().failure();
        thread::sleep(Duration::from_millis(10));

        let probe = health.try_acquire().unwrap();
        assert!(matches!(
            health.try_acquire(),
            Err(Rejected::ProbeLimit { .. })
        ));
        probe.success();
        assert_eq!(
            health.snapshot(),
            CircuitSnapshot::Closed {
                consecutive_failures: 0
            }
        );
    }

    #[test]
    fn failed_probe_reopens_for_a_fresh_cooldown() {
        let health = PassiveHealth::new(config(1, Duration::from_millis(5)));
        health.try_acquire().unwrap().failure();
        thread::sleep(Duration::from_millis(10));
        health.try_acquire().unwrap().failure();

        let CircuitSnapshot::Open { retry_after } = health.snapshot() else {
            panic!("expected reopened circuit");
        };
        assert!(retry_after > Duration::ZERO);
    }

    #[test]
    fn multiple_successful_probes_can_be_required() {
        let config =
            config(1, Duration::from_millis(2)).with_success_threshold(NonZeroU32::new(2).unwrap());
        let health = PassiveHealth::new(config);
        health.try_acquire().unwrap().failure();
        thread::sleep(Duration::from_millis(10));

        health.try_acquire().unwrap().success();
        assert!(matches!(
            health.snapshot(),
            CircuitSnapshot::HalfOpen { successes: 1, .. }
        ));
        health.try_acquire().unwrap().success();
        assert!(matches!(health.snapshot(), CircuitSnapshot::Closed { .. }));
    }

    #[test]
    fn late_result_from_an_old_epoch_is_ignored() {
        let health = PassiveHealth::new(config(1, Duration::from_secs(1)));
        let late_success = health.try_acquire().unwrap();
        health.try_acquire().unwrap().failure();
        late_success.success();

        assert!(matches!(health.snapshot(), CircuitSnapshot::Open { .. }));
    }

    #[test]
    fn force_operations_are_immediate() {
        let health = PassiveHealth::new(config(3, Duration::from_secs(1)));
        health.force_open();
        assert!(!health.is_available());
        health.force_close();
        assert!(health.is_available());
    }

    #[test]
    fn half_open_probe_limit_is_race_safe() {
        const WORKERS: usize = 16;
        let config = config(1, Duration::from_millis(2))
            .with_half_open_max_in_flight(NonZeroU32::new(2).unwrap())
            .with_success_threshold(NonZeroU32::new(3).unwrap());
        let health = PassiveHealth::new(config);
        health.try_acquire().unwrap().failure();
        thread::sleep(Duration::from_millis(10));

        let barrier = Arc::new(Barrier::new(WORKERS));
        let admitted = Arc::new(AtomicU32::new(0));
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let health = health.clone();
                let barrier = Arc::clone(&barrier);
                let admitted = Arc::clone(&admitted);
                thread::spawn(move || {
                    barrier.wait();
                    if let Ok(permit) = health.try_acquire() {
                        admitted.fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(10));
                        permit.cancel();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(admitted.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn zero_open_duration_is_rejected_without_limiting_large_durations() {
        let threshold = NonZeroU32::MIN;
        assert_eq!(
            CircuitConfig::new(threshold, Duration::ZERO),
            Err(CircuitConfigError::ZeroOpenDuration)
        );
        assert!(CircuitConfig::new(threshold, Duration::MAX).is_ok());
    }
}
