use rand::{Rng, SeedableRng, rngs::StdRng};

use crate::{Candidate, PickError, Policy, Selection, Status, Weight};

use super::{
    PriorityCandidate, PriorityConfig, PriorityMode, PriorityWeightedRandom,
    priority::{PriorityScope, member_in_scope, ratio_units},
};

/// A priority candidate assigned to a weighted locality.
///
/// The locality weight is control-plane traffic preference and is distinct
/// from [`Candidate::weight`], which divides a locality's traffic among its
/// endpoints. Every member of one `(priority, locality)` group must advertise
/// the same locality weight.
pub trait LocalityCandidate: PriorityCandidate {
    /// Stable locality identity used to form topology groups.
    type Locality: Clone + Ord;

    /// Returns this candidate's locality.
    fn locality(&self) -> &Self::Locality;

    /// Returns the control-plane weight of this locality.
    fn locality_weight(&self) -> Weight {
        Weight::ONE
    }
}

/// Adds locality metadata to a [`PriorityCandidate`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Localized<C, L> {
    inner: C,
    locality: L,
    locality_weight: Weight,
}

impl<C, L> Localized<C, L> {
    /// Wraps a candidate in a unit-weight locality.
    #[must_use]
    pub const fn new(inner: C, locality: L) -> Self {
        Self {
            inner,
            locality,
            locality_weight: Weight::ONE,
        }
    }

    /// Sets the control-plane weight for this locality.
    #[must_use]
    pub const fn with_locality_weight(mut self, locality_weight: Weight) -> Self {
        self.locality_weight = locality_weight;
        self
    }

    /// Returns the wrapped candidate.
    #[must_use]
    pub const fn inner(&self) -> &C {
        &self.inner
    }

    /// Returns mutable access to the wrapped candidate.
    pub const fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Returns the locality identity.
    #[must_use]
    pub const fn locality(&self) -> &L {
        &self.locality
    }

    /// Returns the control-plane locality weight.
    #[must_use]
    pub const fn locality_weight(&self) -> Weight {
        self.locality_weight
    }

    /// Removes the locality wrapper.
    #[must_use]
    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C, L> Candidate for Localized<C, L>
where
    C: Candidate,
{
    type Id = C::Id;
    type Load = C::Load;

    fn id(&self) -> &Self::Id {
        self.inner.id()
    }

    fn weight(&self) -> Weight {
        self.inner.weight()
    }

    fn load(&self) -> &Self::Load {
        self.inner.load()
    }

    fn status(&self) -> Status {
        self.inner.status()
    }

    fn is_eligible(&self) -> bool {
        self.inner.is_eligible()
    }
}

impl<C, L> PriorityCandidate for Localized<C, L>
where
    C: PriorityCandidate,
{
    fn priority(&self) -> u32 {
        self.inner.priority()
    }

    fn is_priority_member(&self) -> bool {
        self.inner.is_priority_member()
    }

    fn is_panic_eligible(&self) -> bool {
        self.inner.is_panic_eligible()
    }
}

impl<C, L> LocalityCandidate for Localized<C, L>
where
    C: PriorityCandidate,
    L: Clone + Ord,
{
    type Locality = L;

    fn locality(&self) -> &Self::Locality {
        &self.locality
    }

    fn locality_weight(&self) -> Weight {
        self.locality_weight
    }
}

/// A selected endpoint with its priority, locality weight, and health mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LocalityDecision {
    selection: Selection,
    priority: u32,
    priority_mode: PriorityMode,
    locality_weight: Weight,
    effective_locality_weight: u64,
}

impl LocalityDecision {
    /// Returns the selected candidate index.
    #[must_use]
    pub const fn selection(self) -> Selection {
        self.selection
    }

    /// Returns the selected priority number.
    #[must_use]
    pub const fn priority(self) -> u32 {
        self.priority
    }

    /// Returns whether normal or panic eligibility was used.
    #[must_use]
    pub const fn priority_mode(self) -> PriorityMode {
        self.priority_mode
    }

    /// Returns the configured weight of the selected locality.
    #[must_use]
    pub const fn locality_weight(self) -> Weight {
        self.locality_weight
    }

    /// Returns the health-adjusted weight used for locality selection.
    #[must_use]
    pub const fn effective_locality_weight(self) -> u64 {
        self.effective_locality_weight
    }
}

#[derive(Clone, Debug)]
struct LocalityGroup<L> {
    locality: L,
    configured_weight: Weight,
    total_endpoint_weight: u128,
    selected_endpoint_weight: u128,
    effective_weight: u64,
}

/// Health-adjusted weighted routing across localities within a priority.
///
/// Selection follows a strict hierarchy: [`PriorityWeightedRandom`] first
/// chooses a priority and health mode from one sampled eligibility snapshot;
/// locality weights are then scaled by available endpoint capacity; finally,
/// endpoint weights divide the chosen locality's traffic. A healthy locality's
/// capacity loss is absorbed up to the configured overprovisioning factor,
/// with any remaining shortfall spilling proportionally to other localities.
///
/// Calculation uses retained `O(n)` scratch and `O(n log n)` time. Unchanged
/// high-water candidate counts allocate nothing after warmup.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, policy::{Localized, Prioritized, LocalityWeightedRandom}};
///
/// let backends = [
///     Localized::new(Prioritized::new(Backend::new("west"), 0), "us-west"),
///     Localized::new(Prioritized::new(Backend::new("east"), 0), "us-east"),
/// ];
/// let mut policy = LocalityWeightedRandom::seeded(7);
/// let selected = policy.pick(&backends, &())?;
/// assert!(selected.index() < backends.len());
/// # Ok::<(), poise_core::PickError>(())
/// ```
#[derive(Clone, Debug)]
pub struct LocalityWeightedRandom<L, R = StdRng> {
    priority: PriorityWeightedRandom<R>,
    localities: Vec<LocalityGroup<L>>,
}

impl<L> LocalityWeightedRandom<L, StdRng> {
    /// Creates a policy seeded from the process random-number source.
    #[must_use]
    pub fn new(config: PriorityConfig) -> Self {
        Self::with_rng(config, StdRng::from_rng(&mut rand::rng()))
    }

    /// Creates a reproducible policy using the default priority configuration.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self::with_rng(PriorityConfig::default(), StdRng::seed_from_u64(seed))
    }

    /// Creates a reproducible policy using an explicit configuration.
    #[must_use]
    pub fn seeded_with(config: PriorityConfig, seed: u64) -> Self {
        Self::with_rng(config, StdRng::seed_from_u64(seed))
    }
}

impl<L, R> LocalityWeightedRandom<L, R> {
    /// Creates a policy with a caller-provided random-number generator.
    #[must_use]
    pub const fn with_rng(config: PriorityConfig, rng: R) -> Self {
        Self {
            priority: PriorityWeightedRandom::with_rng(config, rng),
            localities: Vec::new(),
        }
    }

    /// Returns the priority and availability configuration.
    #[must_use]
    pub const fn config(&self) -> PriorityConfig {
        self.priority.config()
    }

    /// Returns the random-number generator.
    #[must_use]
    pub const fn rng(&self) -> &R {
        self.priority.rng()
    }

    /// Returns mutable access to the random-number generator.
    pub const fn rng_mut(&mut self) -> &mut R {
        self.priority.rng_mut()
    }

    /// Returns retained priority and locality scratch capacities.
    #[must_use]
    pub fn scratch_capacity(&self) -> ((usize, usize), usize) {
        (self.priority.scratch_capacity(), self.localities.capacity())
    }

    /// Releases retained calculation scratch space.
    pub fn shrink_to_fit(&mut self) {
        self.priority.shrink_to_fit();
        self.localities.shrink_to_fit();
    }

    /// Decomposes the policy into its configuration and random-number generator.
    #[must_use]
    pub fn into_parts(self) -> (PriorityConfig, R) {
        self.priority.into_parts()
    }
}

impl<L> Default for LocalityWeightedRandom<L, StdRng> {
    fn default() -> Self {
        Self::new(PriorityConfig::default())
    }
}

impl<L, R> LocalityWeightedRandom<L, R>
where
    L: Clone + Ord,
    R: Rng,
{
    /// Selects an endpoint and reports the selected topology state.
    ///
    /// # Errors
    ///
    /// In addition to priority-policy errors, returns
    /// [`PickError::InconsistentTopology`] when one locality at one priority
    /// advertises different weights, or [`PickError::WeightOverflow`] when
    /// sampled endpoint weights cannot be represented.
    pub fn decide<C>(&mut self, candidates: &[C]) -> Result<LocalityDecision, PickError>
    where
        C: LocalityCandidate<Locality = L>,
    {
        let scope = self.priority.select_scope(candidates)?;
        self.calculate_localities(candidates, scope)?;

        let total_effective = self.localities.iter().try_fold(0_u64, |total, locality| {
            total
                .checked_add(locality.effective_weight)
                .ok_or(PickError::WeightOverflow)
        })?;
        if total_effective == 0 {
            return Err(PickError::NoEligibleCandidates);
        }

        let mut locality_ticket = self.priority.random_below(total_effective);
        let locality_index = self
            .localities
            .iter()
            .position(|locality| {
                if locality_ticket < locality.effective_weight {
                    true
                } else {
                    locality_ticket -= locality.effective_weight;
                    false
                }
            })
            .ok_or(PickError::NoEligibleCandidates)?;

        let endpoint_total =
            u64::try_from(self.localities[locality_index].selected_endpoint_weight)
                .map_err(|_| PickError::WeightOverflow)?;
        let mut endpoint_ticket = self.priority.random_below(endpoint_total);
        let chosen_locality = &self.localities[locality_index];
        let selected = self
            .priority
            .member_samples()
            .iter()
            .find(|member| {
                if !member_in_scope(**member, scope)
                    || candidates[member.index].locality() != &chosen_locality.locality
                {
                    return false;
                }
                let weight = u64::from(member.weight);
                if endpoint_ticket < weight {
                    true
                } else {
                    endpoint_ticket -= weight;
                    false
                }
            })
            .ok_or(PickError::NoEligibleCandidates)?;

        Ok(LocalityDecision {
            selection: Selection::new(selected.index),
            priority: scope.priority,
            priority_mode: scope.mode,
            locality_weight: chosen_locality.configured_weight,
            effective_locality_weight: chosen_locality.effective_weight,
        })
    }

    fn calculate_localities<C>(
        &mut self,
        candidates: &[C],
        scope: PriorityScope,
    ) -> Result<(), PickError>
    where
        C: LocalityCandidate<Locality = L>,
    {
        self.localities.clear();
        self.localities
            .try_reserve(self.priority.member_samples().len())
            .map_err(|_| PickError::StateCapacityExceeded)?;

        for member in self
            .priority
            .member_samples()
            .iter()
            .filter(|member| member.priority == scope.priority)
        {
            let candidate = &candidates[member.index];
            self.localities.push(LocalityGroup {
                locality: candidate.locality().clone(),
                configured_weight: candidate.locality_weight(),
                total_endpoint_weight: u128::from(member.weight),
                selected_endpoint_weight: if member_in_scope(*member, scope) {
                    u128::from(member.weight)
                } else {
                    0
                },
                effective_weight: 0,
            });
        }
        self.localities
            .sort_unstable_by(|left, right| left.locality.cmp(&right.locality));

        self.localities.dedup_by(|later, earlier| {
            if later.locality != earlier.locality {
                return false;
            }
            if later.configured_weight != earlier.configured_weight {
                // Preserve the conflict for the validation pass below.
                return false;
            }
            earlier.total_endpoint_weight += later.total_endpoint_weight;
            earlier.selected_endpoint_weight += later.selected_endpoint_weight;
            true
        });

        for pair in self.localities.windows(2) {
            if pair[0].locality == pair[1].locality {
                return Err(PickError::InconsistentTopology);
            }
        }

        for locality in &mut self.localities {
            let total = u64::try_from(locality.total_endpoint_weight)
                .map_err(|_| PickError::WeightOverflow)?;
            let selected = u64::try_from(locality.selected_endpoint_weight)
                .map_err(|_| PickError::WeightOverflow)?;
            let mut availability = ratio_units(
                selected,
                total,
                self.priority.config().overprovisioning_factor_percent(),
            );
            // Fixed-point rounding must not make a nonempty locality unreachable.
            if selected > 0 {
                availability = availability.max(1);
            }
            locality.effective_weight = u64::from(locality.configured_weight.get())
                .checked_mul(availability)
                .ok_or(PickError::WeightOverflow)?;
        }
        Ok(())
    }
}

impl<C, Context, L, R> Policy<C, Context> for LocalityWeightedRandom<L, R>
where
    C: LocalityCandidate<Locality = L>,
    Context: ?Sized,
    L: Clone + Ord,
    R: Rng,
{
    fn pick(&mut self, candidates: &[C], _context: &Context) -> Result<Selection, PickError> {
        self.decide(candidates).map(LocalityDecision::selection)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Backend,
        policy::{PanicMode, Prioritized},
    };

    use super::*;

    type TestBackend = Localized<Prioritized<Backend<&'static str>>, &'static str>;

    fn backend(
        id: &'static str,
        priority: u32,
        locality: &'static str,
        locality_weight: u32,
        endpoint_weight: u32,
        status: Status,
    ) -> TestBackend {
        Localized::new(
            Prioritized::new(
                Backend::new(id)
                    .with_weight(Weight::new(endpoint_weight).unwrap())
                    .with_status(status),
                priority,
            ),
            locality,
        )
        .with_locality_weight(Weight::new(locality_weight).unwrap())
    }

    #[test]
    fn wrapper_delegates_candidate_and_priority_metadata() {
        let mut candidate = backend("api", 3, "west", 7, 5, Status::Ready);
        assert_eq!(candidate.id(), &"api");
        assert_eq!(candidate.priority(), 3);
        assert_eq!(candidate.locality(), &"west");
        assert_eq!(candidate.locality_weight().get(), 7);
        assert_eq!(candidate.weight().get(), 5);
        candidate.inner_mut().set_priority(4);
        assert_eq!(candidate.priority(), 4);
        assert_eq!(candidate.into_inner().into_inner().id(), &"api");
    }

    #[test]
    fn healthy_locality_weights_set_traffic_share() {
        let candidates = [
            backend("west", 0, "west", 1, 1, Status::Ready),
            backend("east", 0, "east", 3, 1, Status::Ready),
        ];
        let mut policy = LocalityWeightedRandom::seeded(1);
        let mut west = 0_u32;
        for _ in 0..100_000 {
            west += u32::from(policy.decide(&candidates).unwrap().selection().index() == 0);
        }
        assert!((24_000..26_000).contains(&west), "west={west}");
    }

    #[test]
    fn locality_health_scales_configured_weight() {
        let candidates = [
            backend("x-ready", 0, "x", 1, 1, Status::Ready),
            backend("x-down", 0, "x", 1, 1, Status::Unavailable),
            backend("y-ready", 0, "y", 2, 1, Status::Ready),
        ];
        let mut policy = LocalityWeightedRandom::seeded(2);
        let mut x = 0_u32;
        for _ in 0..100_000 {
            x += u32::from(policy.decide(&candidates).unwrap().selection().index() == 0);
        }
        // x: 1 * 70%; y: 2 * 100% => 70 / 270 = 25.9%.
        assert!((25_000..27_000).contains(&x), "x={x}");
    }

    #[test]
    fn overprovisioning_absorbs_small_locality_loss() {
        let mut candidates = Vec::new();
        for index in 0..7 {
            candidates.push(backend(
                if index < 5 { "x-ready" } else { "x-down" },
                0,
                "x",
                1,
                1,
                if index < 5 {
                    Status::Ready
                } else {
                    Status::Unavailable
                },
            ));
        }
        candidates.push(backend("y", 0, "y", 2, 1, Status::Ready));
        let mut policy = LocalityWeightedRandom::seeded(3);
        let mut x = 0_u32;
        for _ in 0..100_000 {
            let selected = policy.decide(&candidates).unwrap().selection().index();
            x += u32::from(selected < 5);
        }
        assert!((32_000..35_000).contains(&x), "x={x}");
    }

    #[test]
    fn priority_is_selected_before_locality() {
        let candidates = [
            backend("primary", 0, "remote", 1, 1, Status::Ready),
            backend("failover", 1, "local", 1_000, 1, Status::Ready),
        ];
        let mut policy = LocalityWeightedRandom::seeded(4);
        for _ in 0..1_000 {
            let decision = policy.decide(&candidates).unwrap();
            assert_eq!(decision.priority(), 0);
            assert_eq!(decision.selection().index(), 0);
        }
    }

    #[test]
    fn failover_priority_applies_its_own_locality_weights() {
        let candidates = [
            backend("primary", 0, "a", 1, 1, Status::Unavailable),
            backend("secondary-a", 1, "a", 1, 1, Status::Ready),
            backend("secondary-b", 1, "b", 3, 1, Status::Ready),
        ];
        let mut policy = LocalityWeightedRandom::seeded(5);
        let mut b = 0_u32;
        for _ in 0..100_000 {
            let decision = policy.decide(&candidates).unwrap();
            assert_eq!(decision.priority(), 1);
            b += u32::from(decision.selection().index() == 2);
        }
        assert!((74_000..76_000).contains(&b), "b={b}");
    }

    #[test]
    fn panic_uses_panic_eligible_members_in_each_locality() {
        let candidates = [
            backend("a", 0, "a", 1, 1, Status::Unavailable),
            backend("b", 0, "b", 1, 1, Status::Unavailable),
        ];
        let mut policy = LocalityWeightedRandom::seeded(6);
        for _ in 0..100 {
            let decision = policy.decide(&candidates).unwrap();
            assert_eq!(decision.priority_mode(), PriorityMode::Panic);
        }
    }

    #[test]
    fn conflicting_weights_for_one_locality_are_rejected() {
        let candidates = [
            backend("a", 0, "west", 1, 1, Status::Ready),
            backend("b", 0, "west", 2, 1, Status::Ready),
        ];
        assert_eq!(
            LocalityWeightedRandom::seeded(7).decide(&candidates),
            Err(PickError::InconsistentTopology)
        );
    }

    #[test]
    fn endpoint_weights_apply_only_inside_selected_locality() {
        let candidates = [
            backend("small", 0, "west", 1, 1, Status::Ready),
            backend("large", 0, "west", 1, 3, Status::Ready),
        ];
        let mut policy = LocalityWeightedRandom::seeded(8);
        let mut large = 0_u32;
        for _ in 0..100_000 {
            large += u32::from(policy.decide(&candidates).unwrap().selection().index() == 1);
        }
        assert!((74_000..76_000).contains(&large), "large={large}");
    }

    #[test]
    fn seeded_decisions_replay_and_scratch_is_reused() {
        let candidates = [
            backend("a", 0, "a", 1, 1, Status::Ready),
            backend("b", 0, "b", 2, 1, Status::Ready),
            backend("c", 0, "b", 2, 3, Status::Ready),
        ];
        let mut first = LocalityWeightedRandom::seeded(9);
        let expected: Vec<_> = (0..100)
            .map(|_| first.decide(&candidates).unwrap().selection())
            .collect();
        let capacity = first.scratch_capacity();
        let actual: Vec<_> = (0..100)
            .map(|_| first.decide(&candidates).unwrap().selection())
            .collect();
        assert_eq!(capacity, first.scratch_capacity());

        let mut replay = LocalityWeightedRandom::seeded(9);
        let replayed: Vec<_> = (0..200)
            .map(|_| replay.decide(&candidates).unwrap().selection())
            .collect();
        assert_eq!(expected, replayed[..100]);
        assert_eq!(actual, replayed[100..]);
    }

    #[test]
    fn reordering_does_not_change_locality_draws() {
        let original = [
            backend("west", 0, "west", 1, 1, Status::Ready),
            backend("east", 0, "east", 3, 1, Status::Ready),
        ];
        let reordered = [original[1].clone(), original[0].clone()];
        let mut first = LocalityWeightedRandom::seeded(10);
        let mut second = LocalityWeightedRandom::seeded(10);

        for _ in 0..1_000 {
            let first_index = first.decide(&original).unwrap().selection().index();
            let second_index = second.decide(&reordered).unwrap().selection().index();
            assert_eq!(
                original[first_index].locality(),
                reordered[second_index].locality()
            );
        }
    }

    #[test]
    fn standard_empty_fail_closed_and_draining_errors_propagate() {
        let config = PriorityConfig::new(140, 50, PanicMode::FailClosed).unwrap();
        let mut policy = LocalityWeightedRandom::<&str>::seeded_with(config, 11);
        assert_eq!(policy.decide::<TestBackend>(&[]), Err(PickError::Empty));

        let down = [backend("down", 0, "a", 1, 1, Status::Unavailable)];
        assert_eq!(policy.decide(&down), Err(PickError::PanicRejected));

        let draining = [backend("old", 0, "a", 1, 1, Status::Draining)];
        assert_eq!(
            policy.decide(&draining),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
