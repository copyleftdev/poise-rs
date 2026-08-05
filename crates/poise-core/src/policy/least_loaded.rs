use crate::{Candidate, LoadMetric, PickError, Policy, Selection};

use super::no_candidate_error;

/// Selects the eligible candidate with the smallest load metric.
///
/// Ties rotate in slice order to avoid permanently favoring the first backend.
/// Selection takes `O(n)` time and allocates no memory.
#[derive(Clone, Debug, Default)]
pub struct LeastLoaded {
    cursor: usize,
}

impl LeastLoaded {
    /// Creates a policy whose first tie scan starts at index zero.
    #[must_use]
    pub const fn new() -> Self {
        Self { cursor: 0 }
    }
}

impl<C> Policy<C> for LeastLoaded
where
    C: Candidate,
    C::Load: LoadMetric,
{
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        if candidates.is_empty() {
            return Err(PickError::Empty);
        }

        let start = self.cursor % candidates.len();
        let mut selected = None;

        for offset in 0..candidates.len() {
            let index = (start + offset) % candidates.len();
            let candidate = &candidates[index];
            if !candidate.is_eligible() {
                continue;
            }

            let metric = candidate.load().measure();
            match &selected {
                None => selected = Some((index, metric)),
                Some((_, winning_metric)) if metric < *winning_metric => {
                    selected = Some((index, metric));
                }
                Some(_) => {}
            }
        }

        let (index, _) = selected.ok_or_else(|| no_candidate_error(candidates.len()))?;
        self.cursor = index.wrapping_add(1);
        Ok(Selection::new(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status};

    #[test]
    fn selects_smallest_load_and_skips_ineligible() {
        let candidates = [
            Backend::new("a").with_load(5),
            Backend::new("b")
                .with_load(1)
                .with_status(Status::Unavailable),
            Backend::new("c").with_load(2),
        ];
        let mut policy = LeastLoaded::new();

        assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 2);
    }

    #[test]
    fn rotates_equal_load_ties() {
        let candidates = [
            Backend::new("a").with_load(1),
            Backend::new("b").with_load(1),
            Backend::new("c").with_load(1),
        ];
        let mut policy = LeastLoaded::new();

        let indices: Vec<_> = (0..6)
            .map(|_| policy.pick(&candidates, &()).unwrap().index())
            .collect();
        assert_eq!(indices, [0, 1, 2, 0, 1, 2]);
    }
}
