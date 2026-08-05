use rand::{Rng, RngExt, SeedableRng, rngs::StdRng};

use crate::{Candidate, LoadMetric, PickError, Policy, Selection};

use super::no_candidate_error;

/// Samples two eligible candidates uniformly and selects the less loaded one.
///
/// Power of two choices provides strong balancing behavior from approximate
/// load measurements without scanning for the global minimum. This
/// implementation scans once to sample without allocation, so its selection
/// cost is `O(n)` and memory cost is `O(1)`. Ties are broken randomly.
#[derive(Clone, Debug)]
pub struct PowerOfTwoChoices<R = StdRng> {
    rng: R,
}

impl PowerOfTwoChoices<StdRng> {
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

impl<R> PowerOfTwoChoices<R> {
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

impl Default for PowerOfTwoChoices<StdRng> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, R> Policy<C> for PowerOfTwoChoices<R>
where
    C: Candidate,
    C::Load: LoadMetric,
    R: Rng,
{
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        let mut reservoir = [None, None];
        let mut eligible_seen = 0_usize;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            eligible_seen += 1;
            if eligible_seen <= 2 {
                reservoir[eligible_seen - 1] = Some(index);
                continue;
            }

            let slot = self.rng.random_range(0..eligible_seen);
            if slot < 2 {
                reservoir[slot] = Some(index);
            }
        }

        let first = reservoir[0].ok_or_else(|| no_candidate_error(candidates.len()))?;
        let Some(second) = reservoir[1] else {
            return Ok(Selection::new(first));
        };

        let first_load = candidates[first].load().measure();
        let second_load = candidates[second].load().measure();
        let selected = match first_load.cmp(&second_load) {
            std::cmp::Ordering::Less => first,
            std::cmp::Ordering::Greater => second,
            std::cmp::Ordering::Equal => {
                if self.rng.random() {
                    first
                } else {
                    second
                }
            }
        };
        Ok(Selection::new(selected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status};

    #[test]
    fn one_eligible_candidate_is_always_selected() {
        let candidates = [
            Backend::new("a").with_load(0).with_status(Status::Draining),
            Backend::new("b").with_load(100),
        ];
        let mut policy = PowerOfTwoChoices::seeded(13);

        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 1);
    }

    #[test]
    fn globally_worst_candidate_never_wins_a_pair() {
        let candidates = [
            Backend::new("a").with_load(0),
            Backend::new("b").with_load(1),
            Backend::new("c").with_load(2),
        ];
        let mut policy = PowerOfTwoChoices::seeded(14);

        for _ in 0..1_000 {
            assert_ne!(policy.pick(&candidates, &()).unwrap().index(), 2);
        }
    }

    #[test]
    fn seeded_instances_replay_the_same_decisions() {
        let candidates = [
            Backend::new("a").with_load(1),
            Backend::new("b").with_load(1),
            Backend::new("c").with_load(1),
        ];
        let mut left = PowerOfTwoChoices::seeded(15);
        let mut right = PowerOfTwoChoices::seeded(15);

        for _ in 0..100 {
            assert_eq!(left.pick(&candidates, &()), right.pick(&candidates, &()));
        }
    }
}
