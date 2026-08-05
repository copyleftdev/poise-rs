use poise_core::{Candidate, InFlight, Status, Weight};

/// Tower readiness state retained for an endpoint service.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Readiness {
    /// The service must be polled before selection.
    #[default]
    Idle,
    /// `poll_ready` reserved capacity that has not yet been consumed by `call`.
    Ready,
    /// The service returned a readiness error and must be explicitly reset.
    Failed,
}

/// Candidate metadata, a Tower service, and its live dispatch load.
///
/// As a [`Candidate`], an endpoint delegates identity, weight, and
/// administrative eligibility to `C`, exposes `L` as its load metric, and is
/// eligible only while its service owns a readiness reservation.
pub struct Endpoint<C, S, L = InFlight> {
    pub(crate) candidate: C,
    pub(crate) service: S,
    pub(crate) load: L,
    pub(crate) readiness: Readiness,
}

impl<C, S> Endpoint<C, S, InFlight> {
    /// Creates an endpoint with an unbounded shared in-flight counter.
    pub fn new(candidate: C, service: S) -> Self {
        Self::with_tracker(candidate, service, InFlight::new())
    }
}

impl<C, S, L> Endpoint<C, S, L> {
    /// Creates an endpoint with an explicit dispatch load tracker.
    pub const fn with_tracker(candidate: C, service: S, load: L) -> Self {
        Self {
            candidate,
            service,
            load,
            readiness: Readiness::Idle,
        }
    }

    /// Returns the candidate metadata.
    #[must_use]
    pub const fn candidate(&self) -> &C {
        &self.candidate
    }

    /// Returns mutable candidate metadata.
    pub const fn candidate_mut(&mut self) -> &mut C {
        &mut self.candidate
    }

    /// Returns the service without changing its readiness reservation.
    #[must_use]
    pub const fn service(&self) -> &S {
        &self.service
    }

    /// Returns mutable service access after invalidating retained readiness.
    pub fn service_mut(&mut self) -> &mut S {
        self.readiness = Readiness::Idle;
        &mut self.service
    }

    /// Returns the live dispatch load tracker.
    #[must_use]
    pub const fn load_tracker(&self) -> &L {
        &self.load
    }

    /// Returns the retained service readiness state.
    #[must_use]
    pub const fn readiness(&self) -> Readiness {
        self.readiness
    }

    /// Clears a readiness reservation or failure so the service is polled
    /// again.
    pub const fn reset_readiness(&mut self) {
        self.readiness = Readiness::Idle;
    }

    /// Decomposes the endpoint, discarding its transient readiness state.
    pub fn into_parts(self) -> (C, S, L) {
        (self.candidate, self.service, self.load)
    }
}

impl<C, S, L> Candidate for Endpoint<C, S, L>
where
    C: Candidate,
{
    type Id = C::Id;
    type Load = L;

    fn id(&self) -> &Self::Id {
        self.candidate.id()
    }

    fn weight(&self) -> Weight {
        self.candidate.weight()
    }

    fn load(&self) -> &Self::Load {
        &self.load
    }

    fn status(&self) -> Status {
        let status = self.candidate.status();
        if status.is_selectable() && self.readiness != Readiness::Ready {
            Status::Unavailable
        } else {
            status
        }
    }

    fn is_eligible(&self) -> bool {
        self.readiness == Readiness::Ready && self.candidate.is_eligible()
    }
}

impl<C, S, L> std::fmt::Debug for Endpoint<C, S, L>
where
    C: std::fmt::Debug,
    S: std::fmt::Debug,
    L: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("candidate", &self.candidate)
            .field("service", &self.service)
            .field("load", &self.load)
            .field("readiness", &self.readiness)
            .finish()
    }
}
