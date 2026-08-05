use poise_core::{Candidate, Status, Weight};

use crate::{CircuitPermit, PassiveHealth, Rejected};

/// A health source that can admit or exclude new selections.
pub trait HealthSignal {
    /// Returns whether the backend may currently receive a new attempt.
    fn is_available(&self) -> bool;
}

impl HealthSignal for PassiveHealth {
    fn is_available(&self) -> bool {
        self.is_available()
    }
}

/// Composes a backend candidate with a shared health signal.
#[derive(Clone, Debug)]
pub struct HealthChecked<Backend, Health = PassiveHealth> {
    backend: Backend,
    health: Health,
}

impl<Backend, Health> HealthChecked<Backend, Health> {
    /// Wraps a backend with a health signal.
    pub const fn new(backend: Backend, health: Health) -> Self {
        Self { backend, health }
    }

    /// Returns the wrapped backend.
    #[must_use]
    pub const fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Returns mutable access to the wrapped backend.
    pub const fn backend_mut(&mut self) -> &mut Backend {
        &mut self.backend
    }

    /// Returns the shared health handle.
    #[must_use]
    pub const fn health(&self) -> &Health {
        &self.health
    }

    /// Decomposes the wrapper into backend and health state.
    pub fn into_parts(self) -> (Backend, Health) {
        (self.backend, self.health)
    }
}

impl<Backend> HealthChecked<Backend, PassiveHealth> {
    /// Acquires permission to dispatch an attempt.
    ///
    /// # Errors
    ///
    /// Returns [`Rejected`] if the circuit is open or its half-open probe slots
    /// are occupied.
    pub fn try_acquire(&self) -> Result<CircuitPermit, Rejected> {
        self.health.try_acquire()
    }
}

impl<Backend, Health> Candidate for HealthChecked<Backend, Health>
where
    Backend: Candidate,
    Health: HealthSignal,
{
    type Id = Backend::Id;
    type Load = Backend::Load;

    fn id(&self) -> &Self::Id {
        self.backend.id()
    }

    fn weight(&self) -> Weight {
        self.backend.weight()
    }

    fn load(&self) -> &Self::Load {
        self.backend.load()
    }

    fn status(&self) -> Status {
        let status = self.backend.status();
        if status.is_selectable() && !self.health.is_available() {
            Status::Unavailable
        } else {
            status
        }
    }

    fn is_eligible(&self) -> bool {
        self.backend.is_eligible() && self.health.is_available()
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU32, time::Duration};

    use poise_core::{Backend, Policy, Status, policy::RoundRobin};

    use crate::{
        ActiveHealth, ActiveHealthConfig, ActiveStatus, CircuitConfig, PassiveHealth, UnknownPolicy,
    };

    use super::*;

    fn health() -> PassiveHealth {
        PassiveHealth::new(CircuitConfig::new(NonZeroU32::MIN, Duration::from_secs(1)).unwrap())
    }

    #[test]
    fn open_circuit_excludes_only_its_backend() {
        let unhealthy = health();
        let healthy = health();
        unhealthy.try_acquire().unwrap().failure();
        let candidates = [
            HealthChecked::new(Backend::new("a"), unhealthy),
            HealthChecked::new(Backend::new("b"), healthy),
        ];

        assert_eq!(candidates[0].status(), Status::Unavailable);
        let selected = RoundRobin::new().pick(&candidates, &()).unwrap();
        assert_eq!(candidates[selected.index()].id(), &"b");
    }

    #[test]
    fn underlying_unavailability_is_preserved() {
        let candidate =
            HealthChecked::new(Backend::new("a").with_status(Status::Draining), health());

        assert_eq!(candidate.status(), Status::Draining);
        assert!(!candidate.is_eligible());
    }

    #[test]
    fn active_health_is_a_composable_candidate_signal() {
        let config =
            ActiveHealthConfig::new(Duration::from_secs(1), NonZeroU32::MIN, NonZeroU32::MIN)
                .unwrap()
                .with_unknown_policy(UnknownPolicy::Deny);
        let active = ActiveHealth::new(config);
        let candidate = HealthChecked::new(Backend::new("api"), active.clone());

        assert_eq!(candidate.status(), Status::Unavailable);
        active.force_status(ActiveStatus::Healthy);
        assert_eq!(candidate.status(), Status::Ready);
        active.force_status(ActiveStatus::Unhealthy);
        assert_eq!(candidate.status(), Status::Unavailable);
    }

    #[test]
    fn health_wrappers_can_be_nested() {
        let active = ActiveHealth::new(ActiveHealthConfig::default());
        let passive = health();
        let candidate = HealthChecked::new(
            HealthChecked::new(Backend::new("api"), active.clone()),
            passive.clone(),
        );

        assert!(candidate.is_eligible());
        active.force_status(ActiveStatus::Unhealthy);
        assert!(!candidate.is_eligible());
        active.force_status(ActiveStatus::Healthy);
        assert!(candidate.is_eligible());
        passive.try_acquire().unwrap().failure();
        assert!(!candidate.is_eligible());
    }
}
