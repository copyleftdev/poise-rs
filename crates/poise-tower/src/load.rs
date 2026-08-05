use poise_core::{AtCapacity, InFlight, InFlightGuard, LoadMetric, PeakEwma, PeakEwmaGuard};

/// An RAII attempt guard that distinguishes completion from cancellation.
///
/// Returning a response or service error completes the guard. Dropping the
/// response future before it resolves drops the guard without calling this
/// method, allowing trackers such as [`PeakEwma`] to treat it as cancellation.
pub trait LoadGuard {
    /// Records normal completion and releases the tracked attempt.
    fn complete(self);
}

impl LoadGuard for InFlightGuard {
    fn complete(self) {
        self.complete();
    }
}

impl LoadGuard for PeakEwmaGuard {
    fn complete(self) {
        self.complete();
    }
}

/// A live policy load metric that can reserve an outgoing attempt.
pub trait LoadTracker: LoadMetric {
    /// The guard held for the lifetime of the response future.
    type Guard: LoadGuard;

    /// Starts tracking one dispatched attempt.
    ///
    /// # Errors
    ///
    /// Returns [`AtCapacity`] if the tracker cannot reserve another attempt.
    fn start(&self) -> Result<Self::Guard, AtCapacity>;
}

impl LoadTracker for InFlight {
    type Guard = InFlightGuard;

    fn start(&self) -> Result<Self::Guard, AtCapacity> {
        self.start()
    }
}

impl LoadTracker for PeakEwma {
    type Guard = PeakEwmaGuard;

    fn start(&self) -> Result<Self::Guard, AtCapacity> {
        self.start()
    }
}
