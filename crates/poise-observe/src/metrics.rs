use std::{
    array, fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{AttemptEvent, AttemptKind, DecisionEvent, DecisionKind, Observer, ReadinessFailure};

/// Fixed upper bounds for attempt-latency histogram buckets.
///
/// The implicit final bucket has no upper bound. Bounds are process-global and
/// cannot contain user- or backend-provided values, guaranteeing a fixed number
/// of time series when exported.
pub const ATTEMPT_LATENCY_BOUNDS: [Duration; 10] = [
    Duration::from_millis(1),
    Duration::from_millis(5),
    Duration::from_millis(10),
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(5),
];

/// Number of exported latency buckets, including the unbounded final bucket.
pub const ATTEMPT_LATENCY_BUCKET_COUNT: usize = ATTEMPT_LATENCY_BOUNDS.len() + 1;

struct MetricsInner {
    decisions: [AtomicU64; DecisionKind::COUNT],
    attempts: [AtomicU64; AttemptKind::COUNT],
    readiness_failures: AtomicU64,
    latency_buckets: [AtomicU64; ATTEMPT_LATENCY_BUCKET_COUNT],
    latency_micros: AtomicU64,
}

/// Cloneable, lock-free, fixed-cardinality Poise counters.
///
/// The recorder never stores backend identity, request keys, error strings, or
/// policy names. Its complete cardinality is fixed at compile time. Clones
/// share one set of relaxed atomic counters.
#[derive(Clone)]
pub struct Metrics {
    inner: Arc<MetricsInner>,
}

impl Metrics {
    /// Creates zeroed metrics.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsInner {
                decisions: array::from_fn(|_| AtomicU64::new(0)),
                attempts: array::from_fn(|_| AtomicU64::new(0)),
                readiness_failures: AtomicU64::new(0),
                latency_buckets: array::from_fn(|_| AtomicU64::new(0)),
                latency_micros: AtomicU64::new(0),
            }),
        }
    }

    /// Records one policy decision.
    pub fn record_decision(&self, event: DecisionEvent) {
        saturating_add(&self.inner.decisions[event.kind() as usize], 1);
    }

    /// Records one attempt and its elapsed time.
    pub fn record_attempt(&self, event: AttemptEvent) {
        saturating_add(&self.inner.attempts[event.kind() as usize], 1);

        let bucket = ATTEMPT_LATENCY_BOUNDS
            .iter()
            .position(|bound| event.elapsed() <= *bound)
            .unwrap_or(ATTEMPT_LATENCY_BOUNDS.len());
        saturating_add(&self.inner.latency_buckets[bucket], 1);
        let micros = u64::try_from(event.elapsed().as_micros()).unwrap_or(u64::MAX);
        saturating_add(&self.inner.latency_micros, micros);
    }

    /// Records one isolated endpoint readiness failure.
    ///
    /// The endpoint index is deliberately discarded rather than becoming a
    /// metric dimension.
    pub fn record_readiness_failure(&self, _event: ReadinessFailure) {
        saturating_add(&self.inner.readiness_failures, 1);
    }

    /// Samples every counter into an immutable value.
    ///
    /// Individual atomics are read independently. Under concurrent recording,
    /// the snapshot represents a narrow interval rather than a global instant.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let decisions = array::from_fn(|index| self.inner.decisions[index].load(Ordering::Relaxed));
        let attempts = array::from_fn(|index| self.inner.attempts[index].load(Ordering::Relaxed));
        let exact_buckets: [u64; ATTEMPT_LATENCY_BUCKET_COUNT] =
            array::from_fn(|index| self.inner.latency_buckets[index].load(Ordering::Relaxed));
        let mut cumulative = 0_u64;
        let buckets = array::from_fn(|index| {
            cumulative = cumulative.saturating_add(exact_buckets[index]);
            LatencyBucket {
                upper_bound: ATTEMPT_LATENCY_BOUNDS.get(index).copied(),
                count: cumulative,
            }
        });

        MetricsSnapshot {
            decisions,
            attempts,
            readiness_failures: self.inner.readiness_failures.load(Ordering::Relaxed),
            latency: LatencyHistogram {
                buckets,
                sum: Duration::from_micros(self.inner.latency_micros.load(Ordering::Relaxed)),
            },
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Metrics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Metrics")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl Observer for Metrics {
    fn observe_decision(&self, event: DecisionEvent) {
        self.record_decision(event);
    }

    fn observe_attempt(&self, event: AttemptEvent) {
        self.record_attempt(event);
    }

    fn observe_readiness_failure(&self, event: ReadinessFailure) {
        self.record_readiness_failure(event);
    }
}

/// An immutable sample of all bounded metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricsSnapshot {
    decisions: [u64; DecisionKind::COUNT],
    attempts: [u64; AttemptKind::COUNT],
    readiness_failures: u64,
    latency: LatencyHistogram,
}

impl MetricsSnapshot {
    /// Returns decisions in one classification.
    #[must_use]
    pub const fn decisions(self, kind: DecisionKind) -> u64 {
        self.decisions[kind as usize]
    }

    /// Returns attempts in one classification.
    #[must_use]
    pub const fn attempts(self, kind: AttemptKind) -> u64 {
        self.attempts[kind as usize]
    }

    /// Returns the number of isolated readiness failures.
    #[must_use]
    pub const fn readiness_failures(self) -> u64 {
        self.readiness_failures
    }

    /// Returns the cumulative attempt-latency histogram.
    #[must_use]
    pub const fn attempt_latency(self) -> LatencyHistogram {
        self.latency
    }
}

/// One cumulative latency histogram bucket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatencyBucket {
    upper_bound: Option<Duration>,
    count: u64,
}

impl LatencyBucket {
    /// Returns the inclusive upper bound, or `None` for the final bucket.
    #[must_use]
    pub const fn upper_bound(self) -> Option<Duration> {
        self.upper_bound
    }

    /// Returns the cumulative count at or below the upper bound.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.count
    }
}

/// A fixed-bucket cumulative latency histogram.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatencyHistogram {
    buckets: [LatencyBucket; ATTEMPT_LATENCY_BUCKET_COUNT],
    sum: Duration,
}

impl LatencyHistogram {
    /// Returns every cumulative bucket, ending with the unbounded bucket.
    #[must_use]
    pub const fn buckets(self) -> [LatencyBucket; ATTEMPT_LATENCY_BUCKET_COUNT] {
        self.buckets
    }

    /// Returns the total number of observed attempts.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.buckets[ATTEMPT_LATENCY_BUCKET_COUNT - 1].count
    }

    /// Returns the saturating sum of elapsed times at microsecond precision.
    #[must_use]
    pub const fn sum(self) -> Duration {
        self.sum
    }
}

fn saturating_add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(amount))
    });
}

#[cfg(test)]
mod tests {
    /// The documented counter cardinality is the one the type actually owns.
    ///
    /// [`Performance`](../../../docs/performance.md) states a fixed count, and
    /// fixed cardinality is a contract this crate makes rather than an
    /// incidental fact: adding a decision kind or a latency bound changes it
    /// silently, and the prose would still read correctly. Derived here rather
    /// than written down, so the sum has to be updated deliberately.
    #[test]
    fn counter_cardinality_matches_the_documented_total() {
        let total = DecisionKind::COUNT
            + AttemptKind::COUNT
            + 1 // readiness failures
            + ATTEMPT_LATENCY_BUCKET_COUNT
            + 1; // accumulated latency

        assert_eq!(
            total, 28,
            "counter cardinality changed; update the count in docs/performance.md"
        );
    }

    use std::{sync::Arc, thread, time::Duration};

    use super::*;

    #[test]
    fn metrics_use_fixed_dimensions_and_cumulative_latency_buckets() {
        let metrics = Metrics::new();
        metrics.record_decision(DecisionEvent::new(
            DecisionKind::Selected,
            10_000,
            Some(9_999),
        ));
        metrics.record_attempt(AttemptEvent::new(
            AttemptKind::Success,
            Duration::from_millis(4),
        ));
        metrics.record_attempt(AttemptEvent::new(
            AttemptKind::Failure,
            Duration::from_secs(8),
        ));
        for endpoint in 0..10_000 {
            metrics.record_readiness_failure(ReadinessFailure::new(endpoint));
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.decisions(DecisionKind::Selected), 1);
        assert_eq!(snapshot.attempts(AttemptKind::Success), 1);
        assert_eq!(snapshot.attempts(AttemptKind::Failure), 1);
        assert_eq!(snapshot.readiness_failures(), 10_000);

        let buckets = snapshot.attempt_latency().buckets();
        assert_eq!(buckets[0].count(), 0);
        assert_eq!(buckets[1].count(), 1);
        assert_eq!(buckets.last().unwrap().upper_bound(), None);
        assert_eq!(buckets.last().unwrap().count(), 2);
        assert_eq!(
            snapshot.attempt_latency().sum(),
            Duration::from_millis(8_004)
        );
    }

    #[test]
    fn concurrent_clones_share_lossless_counters() {
        const THREADS: usize = 8;
        const ITERATIONS: usize = 5_000;
        let metrics = Arc::new(Metrics::new());
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let metrics = Arc::clone(&metrics);
                thread::spawn(move || {
                    for _ in 0..ITERATIONS {
                        metrics.record_attempt(AttemptEvent::new(
                            AttemptKind::Success,
                            Duration::from_micros(1),
                        ));
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let expected = u64::try_from(THREADS * ITERATIONS).unwrap();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.attempts(AttemptKind::Success), expected);
        assert_eq!(snapshot.attempt_latency().count(), expected);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let metrics = Metrics::new();
        metrics.inner.decisions[DecisionKind::Selected as usize]
            .store(u64::MAX - 1, Ordering::Relaxed);
        let event = DecisionEvent::new(DecisionKind::Selected, 1, Some(0));

        metrics.record_decision(event);
        metrics.record_decision(event);

        assert_eq!(
            metrics.snapshot().decisions(DecisionKind::Selected),
            u64::MAX
        );
    }
}
