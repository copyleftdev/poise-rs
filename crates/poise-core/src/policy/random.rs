use rand::{Rng, RngExt, SeedableRng, rngs::StdRng};

use crate::{Candidate, PickError, Policy, Selection};

use super::no_candidate_error;

/// Selects uniformly among eligible candidates.
///
/// The implementation uses reservoir sampling, taking `O(n)` time and `O(1)`
/// memory without allocating an intermediate eligible-candidate list.
#[derive(Clone, Debug)]
pub struct Random<R = StdRng> {
    rng: R,
}

impl Random<StdRng> {
    /// Creates a policy seeded from the process random-number source.
    #[must_use]
    pub fn new() -> Self {
        Self::with_rng(StdRng::from_rng(&mut rand::rng()))
    }

    /// Creates a reproducible policy from a 64-bit seed.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self::with_rng(StdRng::seed_from_u64(seed))
    }
}

impl<R> Random<R> {
    /// Creates a policy using a caller-provided random-number generator.
    pub const fn with_rng(rng: R) -> Self {
        Self { rng }
    }

    /// Returns a shared reference to the random-number generator.
    pub const fn rng(&self) -> &R {
        &self.rng
    }

    /// Returns a mutable reference to the random-number generator.
    pub const fn rng_mut(&mut self) -> &mut R {
        &mut self.rng
    }

    /// Returns the random-number generator, consuming the policy.
    pub fn into_rng(self) -> R {
        self.rng
    }
}

impl Default for Random<StdRng> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Candidate, R: Rng> Policy<C> for Random<R> {
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        let mut selected = None;
        let mut eligible_seen = 0_usize;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            eligible_seen += 1;
            if self.rng.random_range(0..eligible_seen) == 0 {
                selected = Some(index);
            }
        }

        selected
            .map(Selection::new)
            .ok_or_else(|| no_candidate_error(candidates.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status};

    #[test]
    fn seeded_instances_replay_the_same_decisions() {
        let candidates = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
        let mut left = Random::seeded(42);
        let mut right = Random::seeded(42);

        for _ in 0..100 {
            assert_eq!(left.pick(&candidates, &()), right.pick(&candidates, &()));
        }
    }

    #[test]
    fn never_selects_an_ineligible_candidate() {
        let candidates = [
            Backend::new("a").with_status(Status::Unavailable),
            Backend::new("b"),
            Backend::new("c").with_status(Status::Draining),
        ];
        let mut policy = Random::seeded(7);

        for _ in 0..100 {
            assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 1);
        }
    }

    #[test]
    fn distribution_is_uniform_within_a_generous_bound() {
        let candidates = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
        let mut policy = Random::seeded(9);
        let mut counts = [0_u32; 3];

        for _ in 0..30_000 {
            counts[policy.pick(&candidates, &()).unwrap().index()] += 1;
        }

        for count in counts {
            assert!((9_500..=10_500).contains(&count), "count was {count}");
        }
    }
}
