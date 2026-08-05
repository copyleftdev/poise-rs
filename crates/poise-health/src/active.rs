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

use crate::HealthSignal;

/// Whether an unprobed backend is initially selectable.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum UnknownPolicy {
    /// Allow traffic while active health is unknown.
    #[default]
    Allow,
    /// Require enough healthy probes before allowing traffic.
    Deny,
}

/// Active-health classification.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ActiveStatus {
    /// No threshold has established health yet.
    #[default]
    Unknown,
    /// The healthy threshold was reached most recently.
    Healthy,
    /// The unhealthy threshold was reached most recently.
    Unhealthy,
}

/// The result of one active probe.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ProbeResult {
    /// The probe established a healthy response.
    Healthy,
    /// The probe established an unhealthy response.
    Unhealthy,
}

/// Active-health scheduling and threshold configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActiveHealthConfig {
    interval: Duration,
    healthy_threshold: NonZeroU32,
    unhealthy_threshold: NonZeroU32,
    unknown_policy: UnknownPolicy,
}

impl ActiveHealthConfig {
    /// Creates an active-health configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ActiveHealthConfigError`] when `interval` is zero.
    pub const fn new(
        interval: Duration,
        healthy_threshold: NonZeroU32,
        unhealthy_threshold: NonZeroU32,
    ) -> Result<Self, ActiveHealthConfigError> {
        if interval.is_zero() {
            return Err(ActiveHealthConfigError::ZeroInterval);
        }
        Ok(Self {
            interval,
            healthy_threshold,
            unhealthy_threshold,
            unknown_policy: UnknownPolicy::Allow,
        })
    }

    /// Sets how unknown health affects eligibility.
    #[must_use]
    pub const fn with_unknown_policy(mut self, policy: UnknownPolicy) -> Self {
        self.unknown_policy = policy;
        self
    }

    /// Returns the delay between completed or cancelled probes.
    #[must_use]
    pub const fn interval(self) -> Duration {
        self.interval
    }

    /// Returns successful probes needed to become healthy.
    #[must_use]
    pub const fn healthy_threshold(self) -> NonZeroU32 {
        self.healthy_threshold
    }

    /// Returns failed probes needed to become unhealthy.
    #[must_use]
    pub const fn unhealthy_threshold(self) -> NonZeroU32 {
        self.unhealthy_threshold
    }

    /// Returns the unknown-health eligibility policy.
    #[must_use]
    pub const fn unknown_policy(self) -> UnknownPolicy {
        self.unknown_policy
    }
}

impl Default for ActiveHealthConfig {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(10),
            NonZeroU32::new(2).expect("two is non-zero"),
            NonZeroU32::new(3).expect("three is non-zero"),
        )
        .expect("the default probe interval is non-zero")
    }
}

/// Invalid active-health configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ActiveHealthConfigError {
    /// A zero interval would create an unbounded probe loop.
    ZeroInterval,
}

impl fmt::Display for ActiveHealthConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => f.write_str("active health probe interval must be non-zero"),
        }
    }
}

impl Error for ActiveHealthConfigError {}

struct ActiveState {
    status: ActiveStatus,
    consecutive_healthy: u32,
    consecutive_unhealthy: u32,
    in_flight: bool,
    last_finished: Option<Instant>,
    generation: u64,
}

struct ActiveInner {
    config: ActiveHealthConfig,
    state: Mutex<ActiveState>,
}

/// Executor-neutral active-health scheduling and state.
///
/// The first probe is immediately due. A caller reserves it with
/// [`try_start_probe`](Self::try_start_probe), performs protocol-specific work,
/// and completes the returned [`ActiveProbe`]. Only one probe may run at once.
#[derive(Clone)]
pub struct ActiveHealth {
    inner: Arc<ActiveInner>,
}

impl ActiveHealth {
    /// Creates an unknown active-health state with an immediately due probe.
    #[must_use]
    pub fn new(config: ActiveHealthConfig) -> Self {
        Self {
            inner: Arc::new(ActiveInner {
                config,
                state: Mutex::new(ActiveState {
                    status: ActiveStatus::Unknown,
                    consecutive_healthy: 0,
                    consecutive_unhealthy: 0,
                    in_flight: false,
                    last_finished: None,
                    generation: 0,
                }),
            }),
        }
    }

    /// Reserves the currently due probe.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeRejected`] if another probe is running or the interval
    /// since the previous completion has not elapsed.
    pub fn try_start_probe(&self) -> Result<ActiveProbe, ProbeRejected> {
        self.try_start_probe_at(Instant::now())
    }

    /// Reserves the probe due at a caller-provided monotonic instant.
    ///
    /// Clock-aware adapters and deterministic simulations can use this method
    /// together with [`ActiveProbe::complete_at`] or [`ActiveProbe::cancel_at`].
    /// All supplied instants should come from the same non-decreasing clock.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeRejected`] if another probe is running or the interval
    /// since the previous completion has not elapsed at `now`.
    pub fn try_start_probe_at(&self, now: Instant) -> Result<ActiveProbe, ProbeRejected> {
        let mut state = self.lock_state();
        if state.in_flight {
            return Err(ProbeRejected::Running);
        }
        let due_in = self.due_in(&state, now);
        if !due_in.is_zero() {
            return Err(ProbeRejected::NotDue {
                retry_after: due_in,
            });
        }

        state.in_flight = true;
        Ok(ActiveProbe {
            health: self.clone(),
            generation: state.generation,
            finished: false,
        })
    }

    /// Returns whether a probe can be reserved immediately.
    #[must_use]
    pub fn is_probe_due(&self) -> bool {
        self.is_probe_due_at(Instant::now())
    }

    /// Returns whether a probe can be reserved at a monotonic instant.
    #[must_use]
    pub fn is_probe_due_at(&self, now: Instant) -> bool {
        let state = self.lock_state();
        !state.in_flight && self.due_in(&state, now).is_zero()
    }

    /// Returns a coherent active-health snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ActiveSnapshot {
        self.snapshot_at(Instant::now())
    }

    /// Returns a coherent active-health snapshot at a monotonic instant.
    #[must_use]
    pub fn snapshot_at(&self, now: Instant) -> ActiveSnapshot {
        let state = self.lock_state();
        ActiveSnapshot {
            status: state.status,
            consecutive_healthy: state.consecutive_healthy,
            consecutive_unhealthy: state.consecutive_unhealthy,
            probe_in_flight: state.in_flight,
            due_in: self.due_in(&state, now),
        }
    }

    /// Forces a status, clears threshold counters, and invalidates an existing
    /// probe result.
    pub fn force_status(&self, status: ActiveStatus) {
        let mut state = self.lock_state();
        state.status = status;
        state.consecutive_healthy = 0;
        state.consecutive_unhealthy = 0;
        state.in_flight = false;
        state.generation = state.generation.wrapping_add(1);
    }

    /// Returns the immutable configuration.
    #[must_use]
    pub fn config(&self) -> ActiveHealthConfig {
        self.inner.config
    }

    fn finish_at(&self, generation: u64, result: Option<ProbeResult>, now: Instant) {
        let mut state = self.lock_state();
        if generation != state.generation || !state.in_flight {
            return;
        }

        state.in_flight = false;
        state.last_finished = Some(now);
        let Some(result) = result else {
            return;
        };

        match result {
            ProbeResult::Healthy => {
                state.consecutive_unhealthy = 0;
                state.consecutive_healthy = state.consecutive_healthy.saturating_add(1);
                if state.consecutive_healthy >= self.inner.config.healthy_threshold.get() {
                    state.status = ActiveStatus::Healthy;
                }
            }
            ProbeResult::Unhealthy => {
                state.consecutive_healthy = 0;
                state.consecutive_unhealthy = state.consecutive_unhealthy.saturating_add(1);
                if state.consecutive_unhealthy >= self.inner.config.unhealthy_threshold.get() {
                    state.status = ActiveStatus::Unhealthy;
                }
            }
        }
    }

    fn due_in(&self, state: &ActiveState, now: Instant) -> Duration {
        let Some(last_finished) = state.last_finished else {
            return Duration::ZERO;
        };
        self.inner
            .config
            .interval
            .saturating_sub(now.saturating_duration_since(last_finished))
    }

    fn lock_state(&self) -> MutexGuard<'_, ActiveState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for ActiveHealth {
    fn default() -> Self {
        Self::new(ActiveHealthConfig::default())
    }
}

impl fmt::Debug for ActiveHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveHealth")
            .field("config", &self.config())
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl HealthSignal for ActiveHealth {
    fn is_available(&self) -> bool {
        match self.snapshot().status() {
            ActiveStatus::Healthy => true,
            ActiveStatus::Unhealthy => false,
            ActiveStatus::Unknown => self.inner.config.unknown_policy == UnknownPolicy::Allow,
        }
    }
}

/// A coherent active-health snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveSnapshot {
    status: ActiveStatus,
    consecutive_healthy: u32,
    consecutive_unhealthy: u32,
    probe_in_flight: bool,
    due_in: Duration,
}

impl ActiveSnapshot {
    /// Returns the active-health classification.
    #[must_use]
    pub const fn status(self) -> ActiveStatus {
        self.status
    }

    /// Returns consecutive healthy probe results.
    #[must_use]
    pub const fn consecutive_healthy(self) -> u32 {
        self.consecutive_healthy
    }

    /// Returns consecutive unhealthy probe results.
    #[must_use]
    pub const fn consecutive_unhealthy(self) -> u32 {
        self.consecutive_unhealthy
    }

    /// Returns whether a probe is currently reserved.
    #[must_use]
    pub const fn probe_in_flight(self) -> bool {
        self.probe_in_flight
    }

    /// Returns time until the next probe is due.
    #[must_use]
    pub const fn due_in(self) -> Duration {
        self.due_in
    }
}

/// Why an active probe could not be reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProbeRejected {
    /// Another active probe is outstanding.
    Running,
    /// The configured interval has not elapsed.
    NotDue {
        /// Remaining delay.
        retry_after: Duration,
    },
}

impl ProbeRejected {
    /// Returns a known retry delay.
    #[must_use]
    pub const fn retry_after(self) -> Option<Duration> {
        match self {
            Self::Running => None,
            Self::NotDue { retry_after } => Some(retry_after),
        }
    }
}

impl fmt::Display for ProbeRejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("an active health probe is already running"),
            Self::NotDue { retry_after } => {
                write!(f, "the next active health probe is due in {retry_after:?}")
            }
        }
    }
}

impl Error for ProbeRejected {}

/// An RAII reservation for one active health probe.
pub struct ActiveProbe {
    health: ActiveHealth,
    generation: u64,
    finished: bool,
}

impl ActiveProbe {
    /// Completes the probe with a classified result.
    pub fn complete(mut self, result: ProbeResult) {
        self.finish_at(Some(result), Instant::now());
    }

    /// Completes the probe with a classified result at a monotonic instant.
    ///
    /// `now` should come from the same clock used to reserve this probe.
    pub fn complete_at(mut self, result: ProbeResult, now: Instant) {
        self.finish_at(Some(result), now);
    }

    /// Completes a healthy probe.
    pub fn healthy(self) {
        self.complete(ProbeResult::Healthy);
    }

    /// Completes an unhealthy probe.
    pub fn unhealthy(self) {
        self.complete(ProbeResult::Unhealthy);
    }

    /// Cancels the probe without changing health classification.
    pub fn cancel(mut self) {
        self.finish_at(None, Instant::now());
    }

    /// Cancels the probe at a monotonic instant without changing health
    /// classification.
    ///
    /// `now` should come from the same clock used to reserve this probe.
    pub fn cancel_at(mut self, now: Instant) {
        self.finish_at(None, now);
    }

    fn finish_at(&mut self, result: Option<ProbeResult>, now: Instant) {
        if self.finished {
            return;
        }
        self.health.finish_at(self.generation, result, now);
        self.finished = true;
    }
}

impl fmt::Debug for ActiveProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveProbe")
            .field("generation", &self.generation)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl Drop for ActiveProbe {
    fn drop(&mut self) {
        self.finish_at(None, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU32,
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    const TEST_INTERVAL: Duration = Duration::from_millis(5);

    fn config(
        healthy_threshold: u32,
        unhealthy_threshold: u32,
        unknown_policy: UnknownPolicy,
    ) -> ActiveHealthConfig {
        ActiveHealthConfig::new(
            TEST_INTERVAL,
            NonZeroU32::new(healthy_threshold).unwrap(),
            NonZeroU32::new(unhealthy_threshold).unwrap(),
        )
        .unwrap()
        .with_unknown_policy(unknown_policy)
    }

    fn wait_until_due(health: &ActiveHealth) {
        thread::sleep(TEST_INTERVAL + Duration::from_millis(5));
        assert!(health.is_probe_due());
    }

    #[test]
    fn caller_provided_clock_controls_reservation_and_completion() {
        let health = ActiveHealth::new(config(1, 1, UnknownPolicy::Allow));
        let started = Instant::now();
        health
            .try_start_probe_at(started)
            .unwrap()
            .complete_at(ProbeResult::Healthy, started);

        let almost_due = (started + TEST_INTERVAL)
            .checked_sub(Duration::from_nanos(1))
            .unwrap();
        assert_eq!(
            health.try_start_probe_at(almost_due).unwrap_err(),
            ProbeRejected::NotDue {
                retry_after: Duration::from_nanos(1)
            }
        );
        assert_eq!(
            health.snapshot_at(almost_due).due_in(),
            Duration::from_nanos(1)
        );

        let due = started + TEST_INTERVAL;
        assert!(health.is_probe_due_at(due));
        health.try_start_probe_at(due).unwrap().cancel_at(due);
    }

    #[test]
    fn zero_interval_is_rejected() {
        assert_eq!(
            ActiveHealthConfig::new(Duration::ZERO, NonZeroU32::MIN, NonZeroU32::MIN),
            Err(ActiveHealthConfigError::ZeroInterval)
        );
    }

    #[test]
    fn unknown_policy_controls_initial_eligibility() {
        let allowed = ActiveHealth::new(config(1, 1, UnknownPolicy::Allow));
        let denied = ActiveHealth::new(config(1, 1, UnknownPolicy::Deny));

        assert!(allowed.is_available());
        assert!(!denied.is_available());
    }

    #[test]
    fn thresholds_require_consecutive_results() {
        let health = ActiveHealth::new(config(2, 2, UnknownPolicy::Deny));

        health.try_start_probe().unwrap().unhealthy();
        let snapshot = health.snapshot();
        assert_eq!(snapshot.status(), ActiveStatus::Unknown);
        assert_eq!(snapshot.consecutive_unhealthy(), 1);

        wait_until_due(&health);
        health.try_start_probe().unwrap().healthy();
        let snapshot = health.snapshot();
        assert_eq!(snapshot.status(), ActiveStatus::Unknown);
        assert_eq!(snapshot.consecutive_healthy(), 1);
        assert_eq!(snapshot.consecutive_unhealthy(), 0);

        wait_until_due(&health);
        health.try_start_probe().unwrap().healthy();
        assert_eq!(health.snapshot().status(), ActiveStatus::Healthy);
        assert!(health.is_available());

        wait_until_due(&health);
        health.try_start_probe().unwrap().unhealthy();
        assert_eq!(health.snapshot().status(), ActiveStatus::Healthy);

        wait_until_due(&health);
        health.try_start_probe().unwrap().unhealthy();
        assert_eq!(health.snapshot().status(), ActiveStatus::Unhealthy);
        assert!(!health.is_available());
    }

    #[test]
    fn cancellation_preserves_classification_and_reschedules() {
        let health = ActiveHealth::new(config(1, 1, UnknownPolicy::Allow));
        health.force_status(ActiveStatus::Healthy);

        health.try_start_probe().unwrap().cancel();
        let snapshot = health.snapshot();
        assert_eq!(snapshot.status(), ActiveStatus::Healthy);
        assert_eq!(snapshot.consecutive_healthy(), 0);
        assert_eq!(snapshot.consecutive_unhealthy(), 0);
        assert!(!snapshot.probe_in_flight());
        assert!(!snapshot.due_in().is_zero());
        assert!(matches!(
            health.try_start_probe(),
            Err(ProbeRejected::NotDue { retry_after }) if !retry_after.is_zero()
        ));
    }

    #[test]
    fn only_one_probe_can_be_reserved_concurrently() {
        let workers = 16;
        let health = ActiveHealth::new(config(1, 1, UnknownPolicy::Allow));
        let barrier = Arc::new(Barrier::new(workers));
        let winners = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..workers)
            .map(|_| {
                let health = health.clone();
                let barrier = Arc::clone(&barrier);
                let winners = Arc::clone(&winners);
                thread::spawn(move || {
                    barrier.wait();
                    if let Ok(probe) = health.try_start_probe() {
                        winners.fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(20));
                        probe.cancel();
                    }
                })
            })
            .collect();

        for worker in threads {
            worker.join().unwrap();
        }
        assert_eq!(winners.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn forced_status_invalidates_a_stale_probe_without_touching_a_new_one() {
        let health = ActiveHealth::new(config(1, 1, UnknownPolicy::Allow));
        let stale = health.try_start_probe().unwrap();
        health.force_status(ActiveStatus::Healthy);

        let current = health.try_start_probe().unwrap();
        stale.unhealthy();
        assert!(health.snapshot().probe_in_flight());
        assert_eq!(health.snapshot().status(), ActiveStatus::Healthy);

        current.unhealthy();
        assert!(!health.snapshot().probe_in_flight());
        assert_eq!(health.snapshot().status(), ActiveStatus::Unhealthy);
    }

    #[test]
    fn very_large_intervals_do_not_require_future_instant_arithmetic() {
        let config =
            ActiveHealthConfig::new(Duration::MAX, NonZeroU32::MIN, NonZeroU32::MIN).unwrap();
        let health = ActiveHealth::new(config);

        health.try_start_probe().unwrap().healthy();
        assert!(matches!(
            health.try_start_probe(),
            Err(ProbeRejected::NotDue { retry_after }) if !retry_after.is_zero()
        ));
    }
}
