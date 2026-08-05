use std::{borrow::Borrow, collections::HashMap, error::Error, fmt, hash::Hash, sync::Arc};

use crate::{Discovered, Membership, PublishError, Revision, Snapshot, SnapshotPublisher};

/// A membership change produced by a discovery source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change<Key, Backend> {
    /// Inserts a new member or refreshes and reactivates an existing member.
    Upsert(Key, Backend),
    /// Begins graceful draining for a member.
    Remove(Key),
}

/// The effect of applying a directory operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Effect {
    /// A new member was inserted.
    Inserted,
    /// An active member's backend value was replaced.
    Updated,
    /// A draining member was refreshed and made active again.
    Revived,
    /// Graceful draining began.
    DrainStarted,
    /// The member was already draining; no state changed.
    AlreadyDraining,
    /// Draining finished and the member was physically removed.
    DrainFinished,
    /// The requested member did not exist; no state changed.
    NotFound,
    /// Drain completion was requested for an active member; no state changed.
    NotDraining,
}

impl Effect {
    /// Returns whether the operation changed the directory and its revision.
    #[must_use]
    pub const fn changed(self) -> bool {
        matches!(
            self,
            Self::Inserted
                | Self::Updated
                | Self::Revived
                | Self::DrainStarted
                | Self::DrainFinished
        )
    }
}

/// The effect and resulting revision of a directory operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Applied {
    effect: Effect,
    revision: Revision,
}

impl Applied {
    const fn new(effect: Effect, revision: Revision) -> Self {
        Self { effect, revision }
    }

    /// Returns the operation effect.
    #[must_use]
    pub const fn effect(self) -> Effect {
        self.effect
    }

    /// Returns the directory revision after the operation.
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    /// Returns whether the operation changed the directory.
    #[must_use]
    pub const fn changed(self) -> bool {
        self.effect.changed()
    }
}

struct Entry<Key, Backend> {
    key: Arc<Key>,
    backend: Arc<Backend>,
    membership: Membership,
}

impl<Key, Backend> Clone for Entry<Key, Backend> {
    fn clone(&self) -> Self {
        Self {
            key: Arc::clone(&self.key),
            backend: Arc::clone(&self.backend),
            membership: self.membership,
        }
    }
}

/// A single-writer, insertion-ordered backend membership directory.
///
/// Updates are optimized for correctness and stable snapshot order. Reads
/// should use published [`Snapshot`]s rather than sharing the directory.
pub struct Directory<Key, Backend> {
    revision: Revision,
    entries: Vec<Entry<Key, Backend>>,
    positions: HashMap<Key, usize>,
}

impl<Key, Backend> Directory<Key, Backend>
where
    Key: Clone + Eq + Hash,
{
    /// Creates an empty directory at [`Revision::INITIAL`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            revision: Revision::INITIAL,
            entries: Vec::new(),
            positions: HashMap::new(),
        }
    }

    /// Creates an empty directory at a caller-provided revision.
    ///
    /// This supports restoring a persisted revision clock.
    #[must_use]
    pub fn with_revision(revision: Revision) -> Self {
        Self {
            revision,
            entries: Vec::new(),
            positions: HashMap::new(),
        }
    }

    /// Returns the current revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the number of active and draining members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the directory contains no active or draining members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Applies a discovery change.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionExhausted`] if an effective change would overflow the
    /// revision clock. In that case the directory remains unchanged.
    pub fn apply(&mut self, change: Change<Key, Backend>) -> Result<Applied, RevisionExhausted> {
        match change {
            Change::Upsert(key, backend) => self.upsert(key, backend),
            Change::Remove(key) => self.begin_drain(&key),
        }
    }

    /// Applies a group of discovery changes transactionally.
    ///
    /// The returned outcomes retain their per-change revisions. The directory
    /// commits only when every change succeeds, allowing a caller to publish
    /// one coherent snapshot for a discovery response.
    ///
    /// This operation clones directory metadata and shared backend handles; it
    /// does not clone backend values.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionExhausted`] and leaves the directory unchanged if any
    /// effective change would exhaust the revision clock.
    pub fn apply_batch<I>(&mut self, changes: I) -> Result<Vec<Applied>, RevisionExhausted>
    where
        I: IntoIterator<Item = Change<Key, Backend>>,
    {
        let mut staged = self.clone();
        let outcomes = changes
            .into_iter()
            .map(|change| staged.apply(change))
            .collect::<Result<Vec<_>, _>>()?;
        *self = staged;
        Ok(outcomes)
    }

    /// Inserts, refreshes, or revives a member.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionExhausted`] without mutation if the revision clock is
    /// exhausted.
    pub fn upsert(&mut self, key: Key, backend: Backend) -> Result<Applied, RevisionExhausted> {
        let next = self.next_revision()?;

        let effect = if let Some(&index) = self.positions.get(&key) {
            let entry = &mut self.entries[index];
            let effect = if entry.membership == Membership::Draining {
                Effect::Revived
            } else {
                Effect::Updated
            };
            entry.backend = Arc::new(backend);
            entry.membership = Membership::Active;
            effect
        } else {
            let index = self.entries.len();
            self.positions.insert(key.clone(), index);
            self.entries.push(Entry {
                key: Arc::new(key),
                backend: Arc::new(backend),
                membership: Membership::Active,
            });
            Effect::Inserted
        };

        self.revision = next;
        Ok(Applied::new(effect, self.revision))
    }

    /// Starts graceful draining for a member.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionExhausted`] without mutation if a state change is
    /// needed but the revision clock is exhausted.
    pub fn begin_drain<Q>(&mut self, key: &Q) -> Result<Applied, RevisionExhausted>
    where
        Key: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let Some(&index) = self.positions.get(key) else {
            return Ok(Applied::new(Effect::NotFound, self.revision));
        };
        if self.entries[index].membership == Membership::Draining {
            return Ok(Applied::new(Effect::AlreadyDraining, self.revision));
        }

        let next = self.next_revision()?;
        self.entries[index].membership = Membership::Draining;
        self.revision = next;
        Ok(Applied::new(Effect::DrainStarted, self.revision))
    }

    /// Physically removes a member after its outstanding work has drained.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionExhausted`] without mutation if removal is needed but
    /// the revision clock is exhausted.
    pub fn finish_drain<Q>(&mut self, key: &Q) -> Result<Applied, RevisionExhausted>
    where
        Key: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let Some(&index) = self.positions.get(key) else {
            return Ok(Applied::new(Effect::NotFound, self.revision));
        };
        if self.entries[index].membership != Membership::Draining {
            return Ok(Applied::new(Effect::NotDraining, self.revision));
        }

        let next = self.next_revision()?;
        self.positions.remove(key);
        self.entries.remove(index);
        for (position, entry) in self.entries[index..].iter().enumerate() {
            self.positions
                .insert(entry.key.as_ref().clone(), index + position);
        }
        self.revision = next;
        Ok(Applied::new(Effect::DrainFinished, self.revision))
    }

    /// Creates an immutable snapshot in stable insertion order.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<Discovered<Key, Backend>> {
        let members = self
            .entries
            .iter()
            .map(|entry| {
                Discovered::new(
                    Arc::clone(&entry.key),
                    Arc::clone(&entry.backend),
                    entry.membership,
                )
            })
            .collect();
        Snapshot::new(self.revision, members)
    }

    /// Creates and atomically publishes the current snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError`] if this directory's revision is not newer than
    /// the publisher's current revision.
    pub fn publish(
        &self,
        publisher: &mut SnapshotPublisher<Discovered<Key, Backend>>,
    ) -> Result<Arc<Snapshot<Discovered<Key, Backend>>>, PublishError> {
        publisher.publish(self.snapshot())
    }

    fn next_revision(&self) -> Result<Revision, RevisionExhausted> {
        self.revision.checked_next().ok_or(RevisionExhausted)
    }
}

impl<Key, Backend> Default for Directory<Key, Backend>
where
    Key: Clone + Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Key, Backend> Clone for Directory<Key, Backend>
where
    Key: Clone,
{
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            entries: self.entries.clone(),
            positions: self.positions.clone(),
        }
    }
}

/// The directory's monotonic revision clock cannot advance further.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RevisionExhausted;

impl fmt::Display for RevisionExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the membership revision clock is exhausted")
    }
}

impl Error for RevisionExhausted {}

#[cfg(test)]
mod tests {
    use poise_core::{Backend, Candidate, Policy, Status, policy::RoundRobin};

    use super::*;

    #[test]
    fn removal_drains_before_physical_retirement() {
        let mut directory = Directory::new();
        directory.upsert("a", Backend::new("http://a")).unwrap();
        directory.upsert("b", Backend::new("http://b")).unwrap();

        let applied = directory.begin_drain("a").unwrap();
        assert_eq!(applied.effect(), Effect::DrainStarted);
        assert_eq!(applied.revision(), Revision::new(3));

        let draining = directory.snapshot();
        assert_eq!(draining.len(), 2);
        assert_eq!(draining[0].membership(), Membership::Draining);
        assert_eq!(draining[0].status(), Status::Draining);

        let selected = RoundRobin::new().pick(&draining, &()).unwrap();
        assert_eq!(draining[selected.index()].key(), &"b");

        let applied = directory.finish_drain("a").unwrap();
        assert_eq!(applied.effect(), Effect::DrainFinished);
        let retired = directory.snapshot();
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].key(), &"b");

        // Existing readers keep their coherent pre-retirement snapshot.
        assert_eq!(draining.len(), 2);
        assert_eq!(draining[0].key(), &"a");
    }

    #[test]
    fn rediscovery_revives_a_draining_member_in_place() {
        let mut directory = Directory::new();
        directory.upsert("a", Backend::new(1)).unwrap();
        directory.begin_drain("a").unwrap();

        let applied = directory.upsert("a", Backend::new(2)).unwrap();
        assert_eq!(applied.effect(), Effect::Revived);

        let snapshot = directory.snapshot();
        assert_eq!(snapshot[0].membership(), Membership::Active);
        assert_eq!(snapshot[0].backend().id(), &2);
    }

    #[test]
    fn no_op_lifecycle_requests_do_not_advance_revision() {
        let mut directory: Directory<&str, Backend<usize>> = Directory::new();

        let missing = directory.begin_drain("missing").unwrap();
        assert_eq!(missing.effect(), Effect::NotFound);
        assert!(!missing.changed());
        assert_eq!(missing.revision(), Revision::INITIAL);

        directory.upsert("a", Backend::new(1)).unwrap();
        let active = directory.finish_drain("a").unwrap();
        assert_eq!(active.effect(), Effect::NotDraining);
        assert_eq!(active.revision(), Revision::new(1));

        directory.begin_drain("a").unwrap();
        let repeated = directory.begin_drain("a").unwrap();
        assert_eq!(repeated.effect(), Effect::AlreadyDraining);
        assert_eq!(repeated.revision(), Revision::new(2));
    }

    #[test]
    fn updates_and_retirement_preserve_relative_order() {
        let mut directory = Directory::new();
        for key in ["a", "b", "c", "d"] {
            directory.upsert(key, Backend::new(key)).unwrap();
        }
        directory.upsert("c", Backend::new("c2")).unwrap();
        directory.begin_drain("b").unwrap();
        directory.finish_drain("b").unwrap();

        let keys: Vec<_> = directory
            .snapshot()
            .iter()
            .map(Discovered::key)
            .copied()
            .collect();
        assert_eq!(keys, ["a", "c", "d"]);
    }

    #[test]
    fn revision_overflow_is_transactional() {
        let mut directory: Directory<&str, Backend<usize>> =
            Directory::with_revision(Revision::new(u64::MAX));

        assert_eq!(
            directory.upsert("a", Backend::new(1)),
            Err(RevisionExhausted)
        );
        assert!(directory.is_empty());
        assert_eq!(directory.revision(), Revision::new(u64::MAX));
    }

    #[test]
    fn batch_is_all_or_nothing() {
        let mut directory: Directory<&str, Backend<usize>> =
            Directory::with_revision(Revision::new(u64::MAX - 1));

        let result = directory.apply_batch([
            Change::Upsert("a", Backend::new(1)),
            Change::Upsert("b", Backend::new(2)),
        ]);

        assert_eq!(result, Err(RevisionExhausted));
        assert!(directory.is_empty());
        assert_eq!(directory.revision(), Revision::new(u64::MAX - 1));
    }

    #[test]
    fn batch_reports_each_effect_and_commits_once_complete() {
        let mut directory = Directory::new();
        let outcomes = directory
            .apply_batch([
                Change::Upsert("a", Backend::new(1)),
                Change::Upsert("b", Backend::new(2)),
                Change::Remove("a"),
            ])
            .unwrap();

        let effects: Vec<_> = outcomes.iter().map(|outcome| outcome.effect()).collect();
        assert_eq!(
            effects,
            [Effect::Inserted, Effect::Inserted, Effect::DrainStarted]
        );
        assert_eq!(directory.revision(), Revision::new(3));
        assert_eq!(directory.snapshot()[0].membership(), Membership::Draining);
    }
}
