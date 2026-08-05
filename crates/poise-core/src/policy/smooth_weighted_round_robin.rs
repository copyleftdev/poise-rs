use std::{collections::HashMap, hash::Hash};

use crate::{Candidate, PickError, Policy, Selection};

use super::no_candidate_error;

#[derive(Clone, Copy, Debug, Default)]
struct State {
    current: i128,
    seen_in: u64,
}

/// Smoothly distributes selections in proportion to backend capacity weights.
///
/// Unlike a replicated weighted schedule, this policy spreads high-weight
/// selections throughout the cycle. State is keyed by [`Candidate::Id`], so it
/// follows a backend across snapshot reordering. Removed and ineligible IDs are
/// pruned after each decision; a later reappearance starts with fresh state.
///
/// Selection takes expected `O(n)` time and stores `O(n)` keyed state. Eligible
/// identities must be unique. The identity type is inferred from candidates in
/// typical use:
///
/// ```
/// use poise_core::{Backend, Policy, Weight, policy::SmoothWeightedRoundRobin};
///
/// let backends = [
///     Backend::new("large").with_weight(Weight::new(2).unwrap()),
///     Backend::new("small"),
/// ];
/// let mut policy = SmoothWeightedRoundRobin::new();
/// let selected = policy.pick(&backends, &()).unwrap();
/// assert_eq!(backends[selected.index()].id(), &"large");
/// ```
#[derive(Clone, Debug)]
pub struct SmoothWeightedRoundRobin<Key> {
    states: HashMap<Key, State>,
    epoch: u64,
}

impl<Key> SmoothWeightedRoundRobin<Key> {
    /// Creates an empty smooth weighted policy.
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            epoch: 0,
        }
    }

    /// Returns the number of candidate identities with retained state.
    #[must_use]
    pub fn tracked_len(&self) -> usize {
        self.states.len()
    }

    /// Clears all accumulated scheduling state.
    pub fn reset(&mut self) {
        self.states.clear();
        self.epoch = 0;
    }
}

impl<Key> Default for SmoothWeightedRoundRobin<Key> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Key> Policy<C> for SmoothWeightedRoundRobin<Key>
where
    C: Candidate<Id = Key>,
    Key: Clone + Eq + Hash,
{
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        if candidates.is_empty() {
            self.states.clear();
            return Err(PickError::Empty);
        }

        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Epoch zero is reserved for unseen state. Resetting after 2^64
            // decisions avoids confusing ancient entries with the new epoch.
            self.states.clear();
            self.epoch = 1;
        }

        let mut total_weight = 0_i128;
        let mut winner: Option<(usize, i128)> = None;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            let state = self.states.entry(candidate.id().clone()).or_default();
            if state.seen_in == self.epoch {
                // Do not retain partially advanced scheduling debt after a
                // caller violates the unique-identity contract.
                self.reset();
                return Err(PickError::DuplicateIdentity);
            }
            state.seen_in = self.epoch;

            let weight = i128::from(candidate.weight().get());
            state.current += weight;
            total_weight += weight;
            if winner.is_none_or(|(_, winning_score)| state.current > winning_score) {
                winner = Some((index, state.current));
            }
        }

        self.states.retain(|_, state| state.seen_in == self.epoch);
        let Some((winner_index, _)) = winner else {
            return Err(no_candidate_error(candidates.len()));
        };

        let winner_id = candidates[winner_index].id();
        let winner_state = self
            .states
            .get_mut(winner_id)
            .expect("winner state was retained for the current epoch");
        winner_state.current -= total_weight;
        Ok(Selection::new(winner_index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status, Weight};

    #[test]
    fn produces_exact_long_run_weight_ratios() {
        let candidates = [
            Backend::new("a").with_weight(Weight::new(5).unwrap()),
            Backend::new("b"),
            Backend::new("c"),
        ];
        let mut policy = SmoothWeightedRoundRobin::new();
        let mut counts = [0_u32; 3];

        for _ in 0..700 {
            counts[policy.pick(&candidates, &()).unwrap().index()] += 1;
        }

        assert_eq!(counts, [500, 100, 100]);
    }

    #[test]
    fn high_weight_selections_are_spread_through_the_cycle() {
        let candidates = [
            Backend::new("a").with_weight(Weight::new(5).unwrap()),
            Backend::new("b"),
            Backend::new("c"),
        ];
        let mut policy = SmoothWeightedRoundRobin::new();
        let sequence: Vec<_> = (0..7)
            .map(|_| policy.pick(&candidates, &()).unwrap().index())
            .collect();

        assert_eq!(sequence, [0, 0, 1, 0, 2, 0, 0]);
    }

    #[test]
    fn state_follows_identity_and_prunes_absent_members() {
        let original = [
            Backend::new(String::from("a")).with_weight(Weight::new(2).unwrap()),
            Backend::new(String::from("b")),
        ];
        let reordered = [original[1].clone(), original[0].clone()];
        let only_a = [original[0].clone()];
        let mut policy = SmoothWeightedRoundRobin::new();

        policy.pick(&original, &()).unwrap();
        policy.pick(&reordered, &()).unwrap();
        assert_eq!(policy.tracked_len(), 2);

        policy.pick(&only_a, &()).unwrap();
        assert_eq!(policy.tracked_len(), 1);
    }

    #[test]
    fn ineligible_state_is_pruned() {
        let candidates = [
            Backend::new(String::from("a")),
            Backend::new(String::from("b")),
        ];
        let draining = [
            Backend::new(String::from("a")).with_status(Status::Draining),
            Backend::new(String::from("b")),
        ];
        let mut policy = SmoothWeightedRoundRobin::new();

        policy.pick(&candidates, &()).unwrap();
        policy.pick(&draining, &()).unwrap();
        assert_eq!(policy.tracked_len(), 1);
    }

    #[test]
    fn duplicate_eligible_identity_is_rejected() {
        let candidates = [Backend::new("same"), Backend::new("same")];
        let mut policy = SmoothWeightedRoundRobin::new();

        assert_eq!(
            policy.pick(&candidates, &()),
            Err(PickError::DuplicateIdentity)
        );
    }
}
