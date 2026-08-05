use std::sync::Arc;

use poise_core::{Candidate, Status, Weight};

/// A member's lifecycle within a discovery directory.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Membership {
    /// Discovery currently includes this member.
    #[default]
    Active,
    /// Discovery removed this member, but outstanding work may still refer to
    /// it.
    Draining,
}

/// A backend and its stable discovery identity in an immutable snapshot.
pub struct Discovered<Key, Backend> {
    key: Arc<Key>,
    backend: Arc<Backend>,
    membership: Membership,
}

impl<Key, Backend> Discovered<Key, Backend> {
    pub(crate) const fn new(key: Arc<Key>, backend: Arc<Backend>, membership: Membership) -> Self {
        Self {
            key,
            backend,
            membership,
        }
    }

    /// Returns the stable identity assigned by discovery.
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Returns the discovered backend value.
    #[must_use]
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Returns a shared backend handle.
    #[must_use]
    pub fn backend_arc(&self) -> Arc<Backend> {
        Arc::clone(&self.backend)
    }

    /// Returns the membership lifecycle state.
    #[must_use]
    pub const fn membership(&self) -> Membership {
        self.membership
    }
}

impl<Key, Backend> Clone for Discovered<Key, Backend> {
    fn clone(&self) -> Self {
        Self {
            key: Arc::clone(&self.key),
            backend: Arc::clone(&self.backend),
            membership: self.membership,
        }
    }
}

impl<Key: std::fmt::Debug, Backend: std::fmt::Debug> std::fmt::Debug for Discovered<Key, Backend> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discovered")
            .field("key", &self.key)
            .field("backend", &self.backend)
            .field("membership", &self.membership)
            .finish()
    }
}

impl<Key, Backend> Candidate for Discovered<Key, Backend>
where
    Backend: Candidate,
{
    type Id = Key;
    type Load = Backend::Load;

    fn id(&self) -> &Self::Id {
        self.key()
    }

    fn weight(&self) -> Weight {
        self.backend.weight()
    }

    fn load(&self) -> &Self::Load {
        self.backend.load()
    }

    fn status(&self) -> Status {
        match self.membership {
            Membership::Active => self.backend.status(),
            Membership::Draining => Status::Draining,
        }
    }

    fn is_eligible(&self) -> bool {
        self.membership == Membership::Active && self.backend.is_eligible()
    }
}

#[cfg(test)]
mod tests {
    use poise_core::{Backend, Candidate, Status, Weight};

    use super::*;

    #[test]
    fn delegates_policy_metrics_but_uses_discovery_identity() {
        let backend = Backend::new("transport-id")
            .with_load(7_u32)
            .with_weight(Weight::new(4).unwrap());
        let discovered = Discovered::new(
            Arc::new("discovery-id"),
            Arc::new(backend),
            Membership::Active,
        );

        assert_eq!(discovered.id(), &"discovery-id");
        assert_eq!(discovered.load(), &7);
        assert_eq!(discovered.weight().get(), 4);
        assert_eq!(discovered.status(), Status::Ready);
    }

    #[test]
    fn draining_overrides_backend_eligibility() {
        let discovered = Discovered::new(
            Arc::new("a"),
            Arc::new(Backend::new("a")),
            Membership::Draining,
        );

        assert_eq!(discovered.status(), Status::Draining);
        assert!(!discovered.is_eligible());
    }
}
