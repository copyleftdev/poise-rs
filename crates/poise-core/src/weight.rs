use std::{error::Error, fmt, num::NonZeroU32};

/// A positive backend capacity weight.
///
/// The unit is intentionally unspecified. Only relative weights within one
/// candidate set are meaningful.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Weight(NonZeroU32);

impl Weight {
    /// Unit capacity weight.
    pub const ONE: Self = Self(NonZeroU32::MIN);

    /// Creates a positive weight.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidWeight`] when `value` is zero.
    pub const fn new(value: u32) -> Result<Self, InvalidWeight> {
        match NonZeroU32::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(InvalidWeight),
        }
    }

    /// Returns the integer weight.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl Default for Weight {
    fn default() -> Self {
        Self::ONE
    }
}

impl From<NonZeroU32> for Weight {
    fn from(value: NonZeroU32) -> Self {
        Self(value)
    }
}

impl From<Weight> for NonZeroU32 {
    fn from(value: Weight) -> Self {
        value.0
    }
}

/// Error returned when constructing a zero [`Weight`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidWeight;

impl fmt::Display for InvalidWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a backend weight must be greater than zero")
    }
}

impl Error for InvalidWeight {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero() {
        assert_eq!(Weight::new(0), Err(InvalidWeight));
    }
}
