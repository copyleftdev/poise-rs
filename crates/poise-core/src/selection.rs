/// The result of a successful policy decision.
///
/// An index keeps selection independent from ownership, cloning, and dispatch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Selection {
    index: usize,
}

impl Selection {
    pub(crate) const fn new(index: usize) -> Self {
        Self { index }
    }

    /// Returns the selected index in the candidate slice passed to the policy.
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
}
