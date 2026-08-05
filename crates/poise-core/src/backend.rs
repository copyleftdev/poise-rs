use crate::Weight;

/// Whether a backend may receive new work.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Status {
    /// The backend may receive new work.
    #[default]
    Ready,
    /// The backend is being retired and receives no new work.
    ///
    /// A dispatch layer may keep existing connections or assignments alive,
    /// but the policies in this crate exclude draining candidates.
    Draining,
    /// The backend is known to be unavailable.
    Unavailable,
}

impl Status {
    /// Returns whether the status permits new selections.
    #[must_use]
    pub const fn is_selectable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// A backend as observed by a selection policy.
///
/// Applications may implement this trait for their own snapshot or handle
/// types. [`Backend`] is the batteries-included implementation.
pub trait Candidate {
    /// Stable identity used by keyed policies.
    type Id: ?Sized;

    /// Load metric used by load-aware policies.
    ///
    /// Smaller values represent less load. Use `()` when load is not tracked.
    type Load;

    /// Returns the stable backend identity.
    fn id(&self) -> &Self::Id;

    /// Returns the backend's configured capacity weight.
    fn weight(&self) -> Weight {
        Weight::ONE
    }

    /// Returns the current load metric.
    fn load(&self) -> &Self::Load;

    /// Returns the backend's administrative and health status.
    fn status(&self) -> Status {
        Status::Ready
    }

    /// Returns whether this candidate may receive a new selection.
    ///
    /// Custom implementations may combine status with additional constraints.
    fn is_eligible(&self) -> bool {
        self.status().is_selectable()
    }
}

/// A concrete candidate with identity, user data, load, weight, and status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Backend<Id, Data = (), Load = ()> {
    id: Id,
    data: Data,
    load: Load,
    weight: Weight,
    status: Status,
}

impl<Id> Backend<Id> {
    /// Creates a ready backend with unit weight and no attached data or load.
    pub const fn new(id: Id) -> Self {
        Self {
            id,
            data: (),
            load: (),
            weight: Weight::ONE,
            status: Status::Ready,
        }
    }
}

impl<Id, Data, Load> Backend<Id, Data, Load> {
    /// Creates a backend from all of its parts.
    pub const fn from_parts(
        id: Id,
        data: Data,
        load: Load,
        weight: Weight,
        status: Status,
    ) -> Self {
        Self {
            id,
            data,
            load,
            weight,
            status,
        }
    }

    /// Returns the backend identity.
    pub const fn id(&self) -> &Id {
        &self.id
    }

    /// Returns the application-owned backend data.
    pub const fn data(&self) -> &Data {
        &self.data
    }

    /// Returns mutable access to the application-owned backend data.
    pub const fn data_mut(&mut self) -> &mut Data {
        &mut self.data
    }

    /// Returns the current load metric.
    pub const fn load(&self) -> &Load {
        &self.load
    }

    /// Returns the configured capacity weight.
    pub const fn weight(&self) -> Weight {
        self.weight
    }

    /// Returns the current status.
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Replaces the attached data, preserving the other fields.
    pub fn with_data<NewData>(self, data: NewData) -> Backend<Id, NewData, Load> {
        Backend {
            id: self.id,
            data,
            load: self.load,
            weight: self.weight,
            status: self.status,
        }
    }

    /// Replaces the load metric, preserving the other fields.
    pub fn with_load<NewLoad>(self, load: NewLoad) -> Backend<Id, Data, NewLoad> {
        Backend {
            id: self.id,
            data: self.data,
            load,
            weight: self.weight,
            status: self.status,
        }
    }

    /// Sets the configured capacity weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Sets the current status.
    #[must_use]
    pub const fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    /// Updates the load metric in place.
    pub fn set_load(&mut self, load: Load) {
        self.load = load;
    }

    /// Updates the configured capacity weight in place.
    pub const fn set_weight(&mut self, weight: Weight) {
        self.weight = weight;
    }

    /// Updates the status in place.
    pub const fn set_status(&mut self, status: Status) {
        self.status = status;
    }

    /// Decomposes the backend into identity, data, load, weight, and status.
    pub fn into_parts(self) -> (Id, Data, Load, Weight, Status) {
        (self.id, self.data, self.load, self.weight, self.status)
    }
}

impl<Id, Data, Load> Candidate for Backend<Id, Data, Load> {
    type Id = Id;
    type Load = Load;

    fn id(&self) -> &Self::Id {
        self.id()
    }

    fn weight(&self) -> Weight {
        self.weight()
    }

    fn load(&self) -> &Self::Load {
        self.load()
    }

    fn status(&self) -> Status {
        self.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_preserves_independent_fields() {
        let backend = Backend::new("api")
            .with_data("https://api.example")
            .with_load(7_u32)
            .with_weight(Weight::new(3).unwrap())
            .with_status(Status::Draining);

        assert_eq!(backend.id(), &"api");
        assert_eq!(backend.data(), &"https://api.example");
        assert_eq!(backend.load(), &7);
        assert_eq!(backend.weight().get(), 3);
        assert_eq!(backend.status(), Status::Draining);
        assert!(!backend.is_eligible());
    }
}
