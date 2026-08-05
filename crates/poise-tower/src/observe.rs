/// Observes per-endpoint readiness errors that may be isolated by the pool.
///
/// When another endpoint is ready, [`Balance`](crate::Balance) remains ready
/// and the individual error cannot be returned through Tower's aggregate
/// `poll_ready` result. This hook preserves that diagnostic path without
/// imposing a logging or metrics dependency.
pub trait ObserveReadinessError<E> {
    /// Observes a readiness failure before its error value is returned or
    /// discarded.
    fn observe(&mut self, endpoint: usize, error: &E);
}

/// Discards isolated readiness errors.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct IgnoreReadinessErrors;

impl<E> ObserveReadinessError<E> for IgnoreReadinessErrors {
    fn observe(&mut self, _endpoint: usize, _error: &E) {}
}

impl<E, F> ObserveReadinessError<E> for F
where
    F: FnMut(usize, &E),
{
    fn observe(&mut self, endpoint: usize, error: &E) {
        self(endpoint, error);
    }
}
