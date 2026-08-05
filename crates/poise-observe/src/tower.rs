use poise_tower::ObserveReadinessError;

use crate::{Observer, ReadinessFailure};

/// Adapts a Poise observer to Tower's isolated readiness-error hook.
///
/// The error value is intentionally not retained or converted into a metric
/// label. Applications that need its details can install a closure in Tower
/// that calls this adapter and an application-owned error handler.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TowerObserver<O> {
    observer: O,
}

impl<O> TowerObserver<O> {
    /// Creates a Tower readiness observer.
    #[must_use]
    pub const fn new(observer: O) -> Self {
        Self { observer }
    }

    /// Returns the underlying observer.
    #[must_use]
    pub const fn observer(&self) -> &O {
        &self.observer
    }

    /// Consumes the adapter and returns the underlying observer.
    #[must_use]
    pub fn into_inner(self) -> O {
        self.observer
    }
}

impl<O, E> ObserveReadinessError<E> for TowerObserver<O>
where
    O: Observer,
{
    fn observe(&mut self, endpoint: usize, _error: &E) {
        self.observer
            .observe_readiness_failure(ReadinessFailure::new(endpoint));
    }
}

#[cfg(test)]
mod tests {
    use poise_tower::ObserveReadinessError;

    use crate::Metrics;

    use super::TowerObserver;

    #[test]
    fn adapter_counts_without_partitioning_by_endpoint_or_error() {
        let metrics = Metrics::new();
        let mut observer = TowerObserver::new(metrics.clone());

        observer.observe(1, &"first");
        observer.observe(usize::MAX, &"unbounded user-controlled detail");

        assert_eq!(metrics.snapshot().readiness_failures(), 2);
    }
}
