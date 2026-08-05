use crate::{PickError, Selection};

/// Selects one candidate for a request context.
///
/// The context may be `()` for request-independent policies, or a key/request
/// view for affinity and routing policies.
pub trait Policy<C, Context: ?Sized = ()> {
    /// Selects an eligible index from `candidates`.
    ///
    /// # Errors
    ///
    /// Returns [`PickError`] when no selection can be made.
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError>;
}

/// Convenience operations implemented for every [`Policy`].
pub trait PolicyExt<C, Context: ?Sized = ()>: Policy<C, Context> {
    /// Selects and borrows a candidate directly.
    ///
    /// # Errors
    ///
    /// Returns [`PickError`] when no selection can be made.
    fn choose<'a>(&mut self, candidates: &'a [C], context: &Context) -> Result<&'a C, PickError> {
        let selection = self.pick(candidates, context)?;
        Ok(&candidates[selection.index()])
    }
}

impl<P, C, Context: ?Sized> PolicyExt<C, Context> for P where P: Policy<C, Context> {}
