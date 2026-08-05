use std::hash::{BuildHasher, Hash, Hasher};

use crate::{Candidate, FnvBuildHasher, PickError, Policy, Selection, mix64};

use super::no_candidate_error;

/// Key-affine selection using highest-random-weight (rendezvous) hashing.
///
/// Selection takes `O(n)` time and `O(1)` memory. Adding or removing a backend
/// only remaps keys whose winning relationship changed, making this useful for
/// caches, sharding, and sticky routing.
///
/// The default [`FnvBuildHasher`] is deterministic. It is not collision
/// resistant; use [`Rendezvous::with_hasher`] with an application-appropriate
/// builder when keys are adversarial.
#[derive(Clone, Debug)]
pub struct Rendezvous<S = FnvBuildHasher> {
    hash_builder: S,
}

impl Rendezvous<FnvBuildHasher> {
    /// Creates a deterministic rendezvous policy.
    #[must_use]
    pub fn new() -> Self {
        Self::with_hasher(FnvBuildHasher::default())
    }
}

impl<S> Rendezvous<S> {
    /// Creates a rendezvous policy with a caller-provided hash builder.
    pub const fn with_hasher(hash_builder: S) -> Self {
        Self { hash_builder }
    }

    /// Returns the configured hash builder.
    pub const fn hash_builder(&self) -> &S {
        &self.hash_builder
    }

    /// Returns the hash builder, consuming the policy.
    pub fn into_hash_builder(self) -> S {
        self.hash_builder
    }
}

impl Default for Rendezvous<FnvBuildHasher> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Context, S> Policy<C, Context> for Rendezvous<S>
where
    C: Candidate,
    C::Id: Hash,
    Context: Hash + ?Sized,
    S: BuildHasher,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        let mut winner: Option<(usize, u64)> = None;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            let score = rendezvous_hash(&self.hash_builder, context, candidate.id());

            if winner.is_none_or(|(_, winning_score)| score > winning_score) {
                winner = Some((index, score));
            }
        }

        winner
            .map(|(index, _)| Selection::new(index))
            .ok_or_else(|| no_candidate_error(candidates.len()))
    }
}

pub(super) fn rendezvous_hash<S, Context, Id>(
    hash_builder: &S,
    context: &Context,
    identity: &Id,
) -> u64
where
    S: BuildHasher,
    Context: Hash + ?Sized,
    Id: Hash + ?Sized,
{
    let mut hasher = hash_builder.build_hasher();
    // Domain separators prevent concatenation ambiguity between the request
    // key and backend identity.
    hasher.write_u8(0);
    context.hash(&mut hasher);
    hasher.write_u8(1);
    identity.hash(&mut hasher);
    mix64(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status};

    #[test]
    fn decisions_are_deterministic() {
        let candidates = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
        let mut left = Rendezvous::new();
        let mut right = Rendezvous::new();

        for key in 0..1_000_u64 {
            assert_eq!(left.pick(&candidates, &key), right.pick(&candidates, &key));
        }
    }

    #[test]
    fn removing_a_backend_only_remaps_its_keys() {
        let all = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
        let without_b = [Backend::new("a"), Backend::new("c")];
        let mut policy = Rendezvous::new();

        for key in 0..10_000_u64 {
            let old_id = all[policy.pick(&all, &key).unwrap().index()].id();
            let new_id = without_b[policy.pick(&without_b, &key).unwrap().index()].id();
            if old_id != &"b" {
                assert_eq!(old_id, new_id, "key {key} moved unnecessarily");
            }
        }
    }

    #[test]
    fn excludes_ineligible_winners() {
        let candidates = [
            Backend::new("a").with_status(Status::Unavailable),
            Backend::new("b"),
        ];
        let mut policy = Rendezvous::new();

        for key in 0..100_u64 {
            assert_eq!(policy.pick(&candidates, &key).unwrap().index(), 1);
        }
    }
}
