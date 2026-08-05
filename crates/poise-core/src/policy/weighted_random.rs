use rand::{Rng, RngExt, SeedableRng, rngs::StdRng};

use crate::{Candidate, PickError, Policy, Selection};

use super::no_candidate_error;

/// Selects an eligible candidate with probability proportional to its weight.
///
/// Selection takes two `O(n)` scans, uses `O(1)` memory, and allocates no
/// intermediate table. It is therefore well suited to candidate sets that
/// change frequently; alias tables may be preferable for very large, stable
/// sets.
#[derive(Clone, Debug)]
pub struct WeightedRandom<R = StdRng> {
    rng: R,
}

impl WeightedRandom<StdRng> {
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

impl<R> WeightedRandom<R> {
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

impl Default for WeightedRandom<StdRng> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Candidate, R: Rng> Policy<C> for WeightedRandom<R> {
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        let mut total_weight = 0_u64;
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
        {
            total_weight = total_weight
                .checked_add(u64::from(candidate.weight().get()))
                .ok_or(PickError::WeightOverflow)?;
        }

        if total_weight == 0 {
            return Err(no_candidate_error(candidates.len()));
        }

        let mut ticket = self.rng.random_range(0..total_weight);
        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            let weight = u64::from(candidate.weight().get());
            if ticket < weight {
                return Ok(Selection::new(index));
            }
            ticket -= weight;
        }

        unreachable!("ticket is bounded by the checked sum of eligible weights")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Weight};

    #[test]
    fn distribution_tracks_relative_weights() {
        let candidates = [
            Backend::new("a"),
            Backend::new("b").with_weight(Weight::new(3).unwrap()),
        ];
        let mut policy = WeightedRandom::seeded(11);
        let mut counts = [0_u32; 2];

        for _ in 0..40_000 {
            counts[policy.pick(&candidates, &()).unwrap().index()] += 1;
        }

        assert!((9_500..=10_500).contains(&counts[0]), "counts: {counts:?}");
        assert!((29_500..=30_500).contains(&counts[1]), "counts: {counts:?}");
    }
}
