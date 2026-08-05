use crate::{Candidate, PickError, Policy, Selection};

use super::no_candidate_error;

/// Cycles through eligible candidates in slice order.
///
/// Selection takes `O(n)` time in the worst case and allocates no memory.
/// Reordering the candidate slice also changes the cycle; callers that update
/// membership should preserve a stable order when possible.
#[derive(Clone, Debug, Default)]
pub struct RoundRobin {
    cursor: usize,
}

impl RoundRobin {
    /// Creates a policy whose first scan starts at index zero.
    #[must_use]
    pub const fn new() -> Self {
        Self { cursor: 0 }
    }

    /// Creates a policy whose first scan starts at `cursor` modulo the slice
    /// length.
    #[must_use]
    pub const fn with_cursor(cursor: usize) -> Self {
        Self { cursor }
    }
}

impl<C: Candidate> Policy<C> for RoundRobin {
    fn pick(&mut self, candidates: &[C], _context: &()) -> Result<Selection, PickError> {
        if candidates.is_empty() {
            return Err(PickError::Empty);
        }

        let start = self.cursor % candidates.len();
        for offset in 0..candidates.len() {
            let index = (start + offset) % candidates.len();
            if candidates[index].is_eligible() {
                self.cursor = index.wrapping_add(1);
                return Ok(Selection::new(index));
            }
        }

        Err(no_candidate_error(candidates.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, Status};

    #[test]
    fn cycles_and_skips_ineligible_candidates() {
        let candidates = [
            Backend::new("a"),
            Backend::new("b").with_status(Status::Draining),
            Backend::new("c"),
        ];
        let mut policy = RoundRobin::new();

        let indices: Vec<_> = (0..5)
            .map(|_| policy.pick(&candidates, &()).unwrap().index())
            .collect();

        assert_eq!(indices, [0, 2, 0, 2, 0]);
    }

    #[test]
    fn distinguishes_empty_from_ineligible() {
        let mut policy = RoundRobin::new();
        let empty: [Backend<&str>; 0] = [];
        assert_eq!(policy.pick(&empty, &()), Err(PickError::Empty));

        let candidates = [Backend::new("a").with_status(Status::Unavailable)];
        assert_eq!(
            policy.pick(&candidates, &()),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
