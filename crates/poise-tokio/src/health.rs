use std::{fmt, future::Future, time::Duration};

use poise_health::{ActiveHealth, ActiveProbe, ActiveSnapshot, ProbeRejected, ProbeResult};

struct TokioReservation(Option<ActiveProbe>);

impl TokioReservation {
    fn new(probe: ActiveProbe) -> Self {
        Self(Some(probe))
    }

    fn finish(mut self, result: Option<ProbeResult>) {
        let probe = self.0.take().expect("a reservation is finalized once");
        let now = tokio::time::Instant::now().into_std();
        match result {
            Some(result) => probe.complete_at(result, now),
            None => probe.cancel_at(now),
        }
    }
}

impl Drop for TokioReservation {
    fn drop(&mut self) {
        if let Some(probe) = self.0.take() {
            probe.cancel_at(tokio::time::Instant::now().into_std());
        }
    }
}

/// An asynchronous active-health probe.
///
/// The probe owns the policy that translates transport- or protocol-specific
/// results into a portable [`ProbeResult`].
pub trait TokioProbe {
    /// The future produced for one probe attempt.
    type Future: Future<Output = ProbeResult>;

    /// Starts one probe attempt.
    fn probe(&mut self) -> Self::Future;
}

impl<F, Fut> TokioProbe for F
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ProbeResult>,
{
    type Future = Fut;

    fn probe(&mut self) -> Self::Future {
        self()
    }
}

/// How a timed-out probe affects active-health state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProbeTimeoutPolicy {
    /// Record the timeout as an unhealthy probe result.
    #[default]
    Unhealthy,
    /// Cancel the reservation without changing health classification.
    Cancel,
}

/// Runtime policy for [`ActiveHealthRunner`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeRunnerConfig {
    timeout: Option<Duration>,
    timeout_policy: ProbeTimeoutPolicy,
}

impl ProbeRunnerConfig {
    /// Creates a runner policy with a finite, non-zero timeout.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeRunnerConfigError`] when the duration is zero or cannot
    /// be represented by Tokio's monotonic clock.
    pub fn new(timeout: Duration) -> Result<Self, ProbeRunnerConfigError> {
        if timeout.is_zero() {
            return Err(ProbeRunnerConfigError::ZeroTimeout);
        }

        if tokio::time::Instant::now().checked_add(timeout).is_none() {
            return Err(ProbeRunnerConfigError::TimeoutTooLarge);
        }

        Ok(Self {
            timeout: Some(timeout),
            timeout_policy: ProbeTimeoutPolicy::Unhealthy,
        })
    }

    /// Creates a policy that never times out a started probe.
    ///
    /// Dropping the future returned by [`ActiveHealthRunner::run_once`] still
    /// cancels the health reservation.
    #[must_use]
    pub const fn without_timeout() -> Self {
        Self {
            timeout: None,
            timeout_policy: ProbeTimeoutPolicy::Unhealthy,
        }
    }

    /// Sets the classification applied when a finite timeout expires.
    #[must_use]
    pub const fn with_timeout_policy(mut self, policy: ProbeTimeoutPolicy) -> Self {
        self.timeout_policy = policy;
        self
    }

    /// Returns the configured timeout, or `None` when disabled.
    #[must_use]
    pub const fn timeout(self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the timeout classification policy.
    #[must_use]
    pub const fn timeout_policy(self) -> ProbeTimeoutPolicy {
        self.timeout_policy
    }
}

impl Default for ProbeRunnerConfig {
    fn default() -> Self {
        Self::new(Duration::from_secs(5)).expect("five seconds is a valid Tokio timeout")
    }
}

/// Invalid active-health runner configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeRunnerConfigError {
    /// A zero timeout would deterministically prevent every probe.
    ZeroTimeout,
    /// The duration cannot be represented by Tokio's monotonic clock.
    TimeoutTooLarge,
}

impl fmt::Display for ProbeRunnerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimeout => formatter.write_str("probe timeout must be non-zero"),
            Self::TimeoutTooLarge => formatter.write_str("probe timeout is too large"),
        }
    }
}

impl std::error::Error for ProbeRunnerConfigError {}

/// The result of one scheduled probe attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeReport {
    result: Option<ProbeResult>,
    timed_out: bool,
    snapshot: ActiveSnapshot,
}

impl ProbeReport {
    /// Returns the recorded result, or `None` when a timeout was cancelled.
    #[must_use]
    pub const fn result(self) -> Option<ProbeResult> {
        self.result
    }

    /// Returns whether the probe future reached its deadline.
    #[must_use]
    pub const fn timed_out(self) -> bool {
        self.timed_out
    }

    /// Returns active-health state immediately after finalization.
    #[must_use]
    pub const fn snapshot(self) -> ActiveSnapshot {
        self.snapshot
    }
}

/// Failure to acquire the single active-probe reservation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeRunnerError {
    /// Another caller already owns an active probe for this state machine.
    ProbeAlreadyRunning,
    /// A future version of `poise-health` rejected the reservation for a reason
    /// this adapter does not yet classify specially.
    ReservationRejected(ProbeRejected),
}

impl fmt::Display for ProbeRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProbeAlreadyRunning => {
                formatter.write_str("an active-health probe is already running")
            }
            Self::ReservationRejected(error) => {
                write!(formatter, "probe reservation rejected: {error}")
            }
        }
    }
}

impl std::error::Error for ProbeRunnerError {}

/// Tokio scheduling for a runtime-neutral [`ActiveHealth`] state machine.
///
/// `run_once` waits until the next probe is due, reserves it, runs exactly one
/// attempt, and commits the result. The returned future is cancellation-safe:
/// dropping it while the probe is running drops the reservation and delegates
/// cancellation semantics to [`ActiveHealth`].
#[derive(Debug)]
pub struct ActiveHealthRunner<P> {
    health: ActiveHealth,
    probe: P,
    config: ProbeRunnerConfig,
}

impl<P> ActiveHealthRunner<P> {
    /// Creates a driver for an active-health state machine and probe.
    #[must_use]
    pub const fn new(health: ActiveHealth, probe: P, config: ProbeRunnerConfig) -> Self {
        Self {
            health,
            probe,
            config,
        }
    }

    /// Returns the active-health state machine.
    #[must_use]
    pub const fn health(&self) -> &ActiveHealth {
        &self.health
    }

    /// Returns the probe implementation.
    #[must_use]
    pub const fn probe(&self) -> &P {
        &self.probe
    }

    /// Returns mutable access to the probe implementation.
    #[must_use]
    pub const fn probe_mut(&mut self) -> &mut P {
        &mut self.probe
    }

    /// Returns the runner policy.
    #[must_use]
    pub const fn config(&self) -> ProbeRunnerConfig {
        self.config
    }

    /// Decomposes the driver without discarding either owned component.
    #[must_use]
    pub fn into_parts(self) -> (ActiveHealth, P, ProbeRunnerConfig) {
        (self.health, self.probe, self.config)
    }
}

impl<P> ActiveHealthRunner<P>
where
    P: TokioProbe,
{
    /// Waits for and executes exactly one scheduled probe attempt.
    ///
    /// An externally held reservation is reported instead of being polled in a
    /// busy loop. Calling this method again after that reservation is finalized
    /// resumes normal scheduling.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeRunnerError`] when another caller owns the reservation or
    /// the underlying health state reports a newer rejection variant.
    pub async fn run_once(&mut self) -> Result<ProbeReport, ProbeRunnerError> {
        let reservation = loop {
            let now = tokio::time::Instant::now();
            match self.health.try_start_probe_at(now.into_std()) {
                Ok(probe) => break TokioReservation::new(probe),
                Err(ProbeRejected::NotDue { retry_after }) => {
                    tokio::time::sleep(retry_after).await;
                }
                Err(ProbeRejected::Running) => {
                    return Err(ProbeRunnerError::ProbeAlreadyRunning);
                }
                Err(error) => return Err(ProbeRunnerError::ReservationRejected(error)),
            }
        };

        let outcome = match self.config.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.probe.probe()).await {
                Ok(result) => (Some(result), false),
                Err(_) => match self.config.timeout_policy {
                    ProbeTimeoutPolicy::Unhealthy => (Some(ProbeResult::Unhealthy), true),
                    ProbeTimeoutPolicy::Cancel => (None, true),
                },
            },
            None => (Some(self.probe.probe().await), false),
        };

        reservation.finish(outcome.0);

        Ok(ProbeReport {
            result: outcome.0,
            timed_out: outcome.1,
            snapshot: self
                .health
                .snapshot_at(tokio::time::Instant::now().into_std()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{future, num::NonZeroU32, time::Duration};

    use poise_health::{ActiveHealthConfig, ActiveStatus, ProbeResult};

    use super::{
        ActiveHealthRunner, ProbeRunnerConfig, ProbeRunnerConfigError, ProbeRunnerError,
        ProbeTimeoutPolicy,
    };

    fn health(interval: Duration) -> poise_health::ActiveHealth {
        poise_health::ActiveHealth::new(
            ActiveHealthConfig::new(
                interval,
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(1).unwrap(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn runner_config_rejects_invalid_timeouts() {
        assert_eq!(
            ProbeRunnerConfig::new(Duration::ZERO),
            Err(ProbeRunnerConfigError::ZeroTimeout)
        );
        assert_eq!(
            ProbeRunnerConfig::new(Duration::MAX),
            Err(ProbeRunnerConfigError::TimeoutTooLarge)
        );
    }

    #[tokio::test]
    async fn completed_probe_commits_its_result() {
        let mut runner = ActiveHealthRunner::new(
            health(Duration::from_secs(1)),
            || future::ready(ProbeResult::Healthy),
            ProbeRunnerConfig::default(),
        );

        let report = runner.run_once().await.unwrap();
        assert_eq!(report.result(), Some(ProbeResult::Healthy));
        assert!(!report.timed_out());
        assert_eq!(report.snapshot().status(), ActiveStatus::Healthy);
        assert!(!report.snapshot().probe_in_flight());
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_probes_follow_the_tokio_clock() {
        let interval = Duration::from_secs(11);
        let mut runner = ActiveHealthRunner::new(
            health(interval),
            || future::ready(ProbeResult::Healthy),
            ProbeRunnerConfig::default(),
        );

        runner.run_once().await.unwrap();
        let started = tokio::time::Instant::now();
        runner.run_once().await.unwrap();

        assert_eq!(tokio::time::Instant::now() - started, interval);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_can_be_classified_as_unhealthy() {
        let config = ProbeRunnerConfig::new(Duration::from_secs(3)).unwrap();
        let mut runner =
            ActiveHealthRunner::new(health(Duration::from_secs(1)), future::pending, config);

        let report = runner.run_once().await.unwrap();
        assert_eq!(report.result(), Some(ProbeResult::Unhealthy));
        assert!(report.timed_out());
        assert_eq!(report.snapshot().status(), ActiveStatus::Unhealthy);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_can_cancel_without_changing_classification() {
        let config = ProbeRunnerConfig::new(Duration::from_secs(3))
            .unwrap()
            .with_timeout_policy(ProbeTimeoutPolicy::Cancel);
        let mut runner =
            ActiveHealthRunner::new(health(Duration::from_secs(1)), future::pending, config);

        let report = runner.run_once().await.unwrap();
        assert_eq!(report.result(), None);
        assert!(report.timed_out());
        assert_eq!(report.snapshot().status(), ActiveStatus::Unknown);
        assert_eq!(report.snapshot().consecutive_unhealthy(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_run_once_cancels_its_reservation() {
        let mut runner = ActiveHealthRunner::new(
            health(Duration::from_secs(1)),
            future::pending,
            ProbeRunnerConfig::without_timeout(),
        );

        let outer = tokio::time::timeout(Duration::from_secs(2), runner.run_once()).await;
        assert!(outer.is_err());
        assert!(!runner.health().snapshot().probe_in_flight());
        assert_eq!(runner.health().snapshot().status(), ActiveStatus::Unknown);

        let (state, _, _) = runner.into_parts();
        let mut replacement = ActiveHealthRunner::new(
            state,
            || future::ready(ProbeResult::Healthy),
            ProbeRunnerConfig::default(),
        );
        let started = tokio::time::Instant::now();
        replacement.run_once().await.unwrap();
        assert_eq!(
            tokio::time::Instant::now() - started,
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn competing_reservation_is_reported_without_running_probe() {
        let state = health(Duration::from_secs(1));
        let reservation = state.try_start_probe().unwrap();
        let mut runner = ActiveHealthRunner::new(
            state,
            || future::ready(ProbeResult::Healthy),
            ProbeRunnerConfig::default(),
        );

        assert_eq!(
            runner.run_once().await,
            Err(ProbeRunnerError::ProbeAlreadyRunning)
        );
        reservation.cancel();
    }
}
