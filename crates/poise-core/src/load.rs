use std::{
    cmp::Ordering,
    error::Error,
    fmt,
    num::NonZeroU64,
    time::{Duration, Instant},
};

#[cfg(loom)]
use loom::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering as AtomicOrdering},
};
#[cfg(not(loom))]
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering as AtomicOrdering},
};

/// A source that produces one stable, orderable load sample.
///
/// Selection policies call `measure` at most once per compared candidate. The
/// tracker itself does not need to implement `Ord`, which would be unsound for
/// values that change concurrently.
pub trait LoadMetric {
    /// The immutable value compared by a policy. Smaller means less loaded.
    type Metric: Ord;

    /// Samples the current load.
    fn measure(&self) -> Self::Metric;
}

macro_rules! impl_identity_metric {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl LoadMetric for $ty {
                type Metric = Self;

                fn measure(&self) -> Self::Metric {
                    *self
                }
            }
        )+
    };
}

impl_identity_metric!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    Duration,
);

struct Counter {
    active: AtomicU64,
    limit: Option<NonZeroU64>,
}

/// A shared count of currently active attempts.
///
/// [`InFlight::start`] increments the count and returns an [`InFlightGuard`].
/// Dropping the guard decrements it, including during cancellation and panic
/// unwinding.
///
/// ```
/// use poise_core::{InFlight, LoadMetric};
///
/// let load = InFlight::new();
/// let attempt = load.start().unwrap();
/// assert_eq!(load.measure(), 1);
/// attempt.complete();
/// assert_eq!(load.measure(), 0);
/// ```
#[derive(Clone)]
pub struct InFlight {
    inner: Arc<Counter>,
}

impl InFlight {
    /// Creates an unbounded in-flight counter.
    #[must_use]
    pub fn new() -> Self {
        Self::from_limit(None)
    }

    /// Creates a counter that rejects attempts at `limit`.
    #[must_use]
    pub fn with_limit(limit: NonZeroU64) -> Self {
        Self::from_limit(Some(limit))
    }

    fn from_limit(limit: Option<NonZeroU64>) -> Self {
        Self {
            inner: Arc::new(Counter {
                active: AtomicU64::new(0),
                limit,
            }),
        }
    }

    /// Starts tracking an attempt.
    ///
    /// # Errors
    ///
    /// Returns [`AtCapacity`] when the configured limit is reached or the
    /// unbounded `u64` counter is exhausted.
    pub fn start(&self) -> Result<InFlightGuard, AtCapacity> {
        let mut current = self.inner.active.load(AtomicOrdering::Relaxed);
        loop {
            let unavailable = self.inner.limit.is_some_and(|limit| current >= limit.get());
            if unavailable || current == u64::MAX {
                return Err(AtCapacity {
                    limit: self.inner.limit,
                });
            }

            match self.inner.active.compare_exchange_weak(
                current,
                current + 1,
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
            ) {
                Ok(_) => {
                    return Ok(InFlightGuard {
                        inner: Arc::clone(&self.inner),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Returns the current active-attempt count.
    #[must_use]
    pub fn current(&self) -> u64 {
        self.inner.active.load(AtomicOrdering::Relaxed)
    }

    /// Returns the configured limit, or `None` for an unbounded counter.
    #[must_use]
    pub fn limit(&self) -> Option<NonZeroU64> {
        self.inner.limit
    }
}

impl Default for InFlight {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InFlight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InFlight")
            .field("current", &self.current())
            .field("limit", &self.limit())
            .finish()
    }
}

impl LoadMetric for InFlight {
    type Metric = u64;

    fn measure(&self) -> Self::Metric {
        self.current()
    }
}

/// An RAII token representing one active attempt.
pub struct InFlightGuard {
    inner: Arc<Counter>,
}

impl InFlightGuard {
    /// Completes the attempt.
    ///
    /// This is equivalent to dropping the guard and is provided for expressive
    /// call sites.
    pub fn complete(self) {}
}

impl fmt::Debug for InFlightGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InFlightGuard").finish_non_exhaustive()
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let previous = self.inner.active.fetch_sub(1, AtomicOrdering::Relaxed);
        debug_assert!(previous > 0, "an in-flight guard decremented below zero");
    }
}

/// An attempt could not enter an in-flight tracker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AtCapacity {
    limit: Option<NonZeroU64>,
}

impl AtCapacity {
    /// Returns the configured limit, or `None` if the raw counter was exhausted.
    #[must_use]
    pub const fn limit(self) -> Option<NonZeroU64> {
        self.limit
    }
}

impl fmt::Display for AtCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.limit {
            Some(limit) => write!(f, "the in-flight limit of {limit} has been reached"),
            None => f.write_str("the in-flight counter is exhausted"),
        }
    }
}

impl Error for AtCapacity {}

/// A totally ordered load score measured in latency-seconds times concurrency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadScore(f64);

impl LoadScore {
    fn new(value: f64) -> Self {
        debug_assert!(value.is_finite() && value >= 0.0);
        Self(value)
    }

    /// Returns the numeric score.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Eq for LoadScore {}

impl Ord for LoadScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for LoadScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct EwmaState {
    cost_seconds: f64,
    updated_at: Instant,
}

struct PeakInner {
    in_flight: InFlight,
    default_rtt: Duration,
    half_life: Duration,
    state: Mutex<EwmaState>,
}

/// A peak exponentially weighted moving-average latency tracker.
///
/// Observed latency raises the estimate immediately; the estimate then decays
/// toward `default_rtt` with the configured half-life. The load score multiplies
/// that estimate by `in_flight + 1`, combining recent slowness and concurrency.
/// Clones share all state.
///
/// ```
/// use std::time::Duration;
/// use poise_core::{LoadMetric, PeakEwma};
///
/// let load = PeakEwma::new(Duration::from_millis(10), Duration::from_secs(10))?;
/// load.observe(Duration::from_millis(50));
/// assert!(load.measure().get() > 0.04);
/// # Ok::<(), poise_core::PeakEwmaConfigError>(())
/// ```
#[derive(Clone)]
pub struct PeakEwma {
    inner: Arc<PeakInner>,
}

impl PeakEwma {
    /// Creates an unbounded tracker.
    ///
    /// # Errors
    ///
    /// Returns [`PeakEwmaConfigError`] if `default_rtt` or `half_life` is zero.
    pub fn new(default_rtt: Duration, half_life: Duration) -> Result<Self, PeakEwmaConfigError> {
        Self::from_parts(default_rtt, half_life, InFlight::new())
    }

    /// Creates a tracker with a maximum number of concurrent attempts.
    ///
    /// # Errors
    ///
    /// Returns [`PeakEwmaConfigError`] if `default_rtt` or `half_life` is zero.
    pub fn with_limit(
        default_rtt: Duration,
        half_life: Duration,
        limit: NonZeroU64,
    ) -> Result<Self, PeakEwmaConfigError> {
        Self::from_parts(default_rtt, half_life, InFlight::with_limit(limit))
    }

    fn from_parts(
        default_rtt: Duration,
        half_life: Duration,
        in_flight: InFlight,
    ) -> Result<Self, PeakEwmaConfigError> {
        if default_rtt.is_zero() {
            return Err(PeakEwmaConfigError::ZeroDefaultRtt);
        }
        if half_life.is_zero() {
            return Err(PeakEwmaConfigError::ZeroHalfLife);
        }

        Ok(Self {
            inner: Arc::new(PeakInner {
                in_flight,
                default_rtt,
                half_life,
                state: Mutex::new(EwmaState {
                    cost_seconds: default_rtt.as_secs_f64(),
                    updated_at: Instant::now(),
                }),
            }),
        })
    }

    /// Starts timing and tracking an attempt.
    ///
    /// Call [`PeakEwmaGuard::complete`] to record response latency. Dropping the
    /// guard without completion is treated as cancellation: concurrency is
    /// released without adding a latency sample.
    ///
    /// # Errors
    ///
    /// Returns [`AtCapacity`] when the configured pending limit is reached.
    pub fn start(&self) -> Result<PeakEwmaGuard, AtCapacity> {
        let in_flight = self.inner.in_flight.start()?;
        Ok(PeakEwmaGuard {
            tracker: self.clone(),
            started_at: Instant::now(),
            in_flight: Some(in_flight),
            finished: false,
        })
    }

    /// Records a latency sample without creating an attempt guard.
    pub fn observe(&self, latency: Duration) {
        let now = Instant::now();
        let mut state = self.lock_state();
        let decayed = self.decayed_cost(&state, now);
        state.cost_seconds = decayed.max(latency.as_secs_f64());
        state.updated_at = now;
    }

    /// Returns the active-attempt count.
    #[must_use]
    pub fn in_flight(&self) -> u64 {
        self.inner.in_flight.current()
    }

    /// Returns the current decayed latency estimate.
    #[must_use]
    pub fn estimated_rtt(&self) -> Duration {
        Duration::from_secs_f64(self.estimated_seconds(Instant::now()))
    }

    /// Returns the current load score.
    #[must_use]
    pub fn score(&self) -> LoadScore {
        let cost = self.estimated_seconds(Instant::now());
        // Conversion remains monotonic above 2^53 even though adjacent counts
        // can share a representation. Exact unit precision at that concurrency
        // is neither representable nor operationally meaningful.
        #[allow(clippy::cast_precision_loss)]
        let concurrency = self.in_flight() as f64 + 1.0;
        LoadScore::new(cost * concurrency)
    }

    /// Returns the configured default latency floor.
    #[must_use]
    pub fn default_rtt(&self) -> Duration {
        self.inner.default_rtt
    }

    /// Returns the configured decay half-life.
    #[must_use]
    pub fn half_life(&self) -> Duration {
        self.inner.half_life
    }

    fn estimated_seconds(&self, now: Instant) -> f64 {
        let mut state = self.lock_state();
        state.cost_seconds = self.decayed_cost(&state, now);
        state.updated_at = now;
        state.cost_seconds
    }

    fn decayed_cost(&self, state: &EwmaState, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(state.updated_at);
        let half_lives = elapsed.as_secs_f64() / self.inner.half_life.as_secs_f64();
        let factor = f64::exp2(-half_lives);
        (state.cost_seconds * factor).max(self.inner.default_rtt.as_secs_f64())
    }

    fn lock_state(&self) -> MutexGuard<'_, EwmaState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for PeakEwma {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeakEwma")
            .field("in_flight", &self.in_flight())
            .field("estimated_rtt", &self.estimated_rtt())
            .field("default_rtt", &self.default_rtt())
            .field("half_life", &self.half_life())
            .finish()
    }
}

impl LoadMetric for PeakEwma {
    type Metric = LoadScore;

    fn measure(&self) -> Self::Metric {
        self.score()
    }
}

/// An RAII token that records attempt duration and concurrency.
pub struct PeakEwmaGuard {
    tracker: PeakEwma,
    started_at: Instant,
    in_flight: Option<InFlightGuard>,
    finished: bool,
}

impl PeakEwmaGuard {
    /// Completes the attempt and records its elapsed time.
    pub fn complete(mut self) {
        self.finish(true);
    }

    /// Cancels the attempt without recording a latency sample.
    ///
    /// This is equivalent to dropping the guard without completing it.
    pub fn cancel(mut self) {
        self.finish(false);
    }

    fn finish(&mut self, observe: bool) {
        if self.finished {
            return;
        }

        if observe {
            self.tracker.observe(self.started_at.elapsed());
        }
        // Completion publishes the latency before reducing concurrency. A
        // racing selector therefore sees a conservative combination rather
        // than stale latency with a prematurely lower pending count.
        drop(self.in_flight.take());
        self.finished = true;
    }
}

impl fmt::Debug for PeakEwmaGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeakEwmaGuard")
            .field("elapsed", &self.started_at.elapsed())
            .finish_non_exhaustive()
    }
}

impl Drop for PeakEwmaGuard {
    fn drop(&mut self) {
        self.finish(false);
    }
}

/// Invalid peak-EWMA configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PeakEwmaConfigError {
    /// A zero default latency would make cold backends unrealistically free.
    ZeroDefaultRtt,
    /// A zero half-life makes exponential decay undefined.
    ZeroHalfLife,
}

impl fmt::Display for PeakEwmaConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDefaultRtt => f.write_str("peak EWMA default RTT must be non-zero"),
            Self::ZeroHalfLife => f.write_str("peak EWMA half-life must be non-zero"),
        }
    }
}

impl Error for PeakEwmaConfigError {}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU64, sync::Barrier, thread};

    use crate::{Backend, Policy, policy::LeastLoaded};

    use super::*;

    #[test]
    fn in_flight_guards_balance_on_complete_and_drop() {
        let tracker = InFlight::new();
        let first = tracker.start().unwrap();
        assert_eq!(tracker.current(), 1);

        {
            let second = tracker.start().unwrap();
            assert_eq!(tracker.current(), 2);
            second.complete();
        }
        assert_eq!(tracker.current(), 1);

        drop(first);
        assert_eq!(tracker.current(), 0);
    }

    #[test]
    fn in_flight_limit_rejects_without_incrementing() {
        let limit = NonZeroU64::new(2).unwrap();
        let tracker = InFlight::with_limit(limit);
        let _first = tracker.start().unwrap();
        let _second = tracker.start().unwrap();

        let error = tracker.start().unwrap_err();
        assert_eq!(error.limit(), Some(limit));
        assert_eq!(tracker.current(), 2);
    }

    #[test]
    fn shared_counter_is_correct_under_concurrency() {
        const WORKERS: usize = 8;
        let tracker = InFlight::new();
        let barrier = Arc::new(Barrier::new(WORKERS + 1));
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let tracker = tracker.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let _guard = tracker.start().unwrap();
                    barrier.wait();
                    barrier.wait();
                })
            })
            .collect();

        barrier.wait();
        assert_eq!(tracker.current(), WORKERS as u64);
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(tracker.current(), 0);
    }

    #[test]
    fn least_loaded_samples_live_in_flight_counts() {
        let a = InFlight::new();
        let b = InFlight::new();
        let c = InFlight::new();
        let candidates = [
            Backend::new("a").with_load(a.clone()),
            Backend::new("b").with_load(b.clone()),
            Backend::new("c").with_load(c.clone()),
        ];
        let _a1 = a.start().unwrap();
        let _a2 = a.start().unwrap();
        let _b1 = b.start().unwrap();

        let selected = LeastLoaded::new().pick(&candidates, &()).unwrap();
        assert_eq!(candidates[selected.index()].id(), &"c");
    }

    #[test]
    fn peak_ewma_rejects_zero_time_parameters() {
        assert!(matches!(
            PeakEwma::new(Duration::ZERO, Duration::from_secs(1)),
            Err(PeakEwmaConfigError::ZeroDefaultRtt)
        ));
        assert!(matches!(
            PeakEwma::new(Duration::from_millis(1), Duration::ZERO),
            Err(PeakEwmaConfigError::ZeroHalfLife)
        ));
    }

    #[test]
    fn peak_ewma_latency_rises_immediately_and_decays_to_floor() {
        let tracker = PeakEwma::new(Duration::from_millis(1), Duration::from_millis(10)).unwrap();
        tracker.observe(Duration::from_millis(64));
        let peak = tracker.estimated_rtt();
        assert!(peak >= Duration::from_millis(32), "peak was {peak:?}");

        thread::sleep(Duration::from_millis(100));
        let decayed = tracker.estimated_rtt();
        assert!(
            decayed < Duration::from_millis(10),
            "decayed to {decayed:?}"
        );
        assert!(decayed >= Duration::from_millis(1));
    }

    #[test]
    fn peak_ewma_score_includes_concurrency() {
        let tracker = PeakEwma::new(Duration::from_millis(10), Duration::from_secs(60)).unwrap();
        let idle = tracker.score().get();
        let _guard = tracker.start().unwrap();
        let busy = tracker.score().get();

        assert!(busy > idle * 1.9, "idle={idle}, busy={busy}");
    }

    #[test]
    fn completion_records_latency_and_releases_capacity() {
        let tracker = PeakEwma::new(Duration::from_nanos(1), Duration::from_secs(1)).unwrap();
        let guard = tracker.start().unwrap();
        assert_eq!(tracker.in_flight(), 1);
        thread::sleep(Duration::from_millis(2));
        guard.complete();

        assert_eq!(tracker.in_flight(), 0);
        assert!(tracker.estimated_rtt() > Duration::from_millis(1));
    }

    #[test]
    fn cancellation_releases_capacity_without_recording_latency() {
        let default_rtt = Duration::from_millis(1);
        let tracker = PeakEwma::new(default_rtt, Duration::from_secs(60)).unwrap();
        let guard = tracker.start().unwrap();
        thread::sleep(Duration::from_millis(2));
        drop(guard);

        assert_eq!(tracker.in_flight(), 0);
        assert!(tracker.estimated_rtt() < Duration::from_millis(2));
    }

    #[test]
    fn least_loaded_uses_peak_ewma_scores() {
        let slow = PeakEwma::new(Duration::from_millis(1), Duration::from_secs(60)).unwrap();
        let fast = PeakEwma::new(Duration::from_millis(1), Duration::from_secs(60)).unwrap();
        slow.observe(Duration::from_millis(100));
        let candidates = [
            Backend::new("slow").with_load(slow),
            Backend::new("fast").with_load(fast),
        ];

        let selected = LeastLoaded::new().pick(&candidates, &()).unwrap();
        assert_eq!(candidates[selected.index()].id(), &"fast");
    }
}
