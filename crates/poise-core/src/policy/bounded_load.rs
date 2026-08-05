use std::{
    error::Error,
    fmt,
    hash::{BuildHasher, Hash},
    num::NonZeroU32,
};

use crate::{Candidate, FnvBuildHasher, LoadMetric, PickError, Policy, Selection};

use super::{
    no_candidate_error,
    rendezvous::rendezvous_hash,
    weighted_rendezvous::{exponential_score, weighted_score_wins},
};

/// Default maximum load as a percentage of weighted-average load.
pub const DEFAULT_BALANCE_FACTOR_PERCENT: u32 = 150;

/// Load-bound configuration for [`BoundedLoadRendezvous`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundedLoadConfig {
    balance_factor_percent: NonZeroU32,
}

impl BoundedLoadConfig {
    /// Creates a configuration from a percentage of weighted-average load.
    ///
    /// A value of `100` is the tightest valid bound; `150` allows each
    /// candidate to carry up to 1.5 times its weighted share.
    ///
    /// # Errors
    ///
    /// Returns [`BoundedLoadConfigError::BelowAverage`] below 100 percent.
    pub fn new(balance_factor_percent: u32) -> Result<Self, BoundedLoadConfigError> {
        if balance_factor_percent < 100 {
            return Err(BoundedLoadConfigError::BelowAverage);
        }
        let Some(balance_factor_percent) = NonZeroU32::new(balance_factor_percent) else {
            return Err(BoundedLoadConfigError::BelowAverage);
        };
        Ok(Self {
            balance_factor_percent,
        })
    }

    /// Returns the bound as a percentage of weighted-average load.
    #[must_use]
    pub const fn balance_factor_percent(self) -> NonZeroU32 {
        self.balance_factor_percent
    }
}

impl Default for BoundedLoadConfig {
    fn default() -> Self {
        Self {
            balance_factor_percent: NonZeroU32::new(DEFAULT_BALANCE_FACTOR_PERCENT)
                .expect("the default balance factor is non-zero"),
        }
    }
}

/// Invalid bounded-load configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoundedLoadConfigError {
    /// A factor below average cannot guarantee a candidate has spare capacity.
    BelowAverage,
}

impl fmt::Display for BoundedLoadConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded-load balance factor must be at least 100 percent")
    }
}

impl Error for BoundedLoadConfigError {}

/// An affinity decision and the load bound that produced it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundedLoadDecision {
    selection: Selection,
    affinity: Selection,
    selected_load: u64,
    selected_capacity: u128,
}

impl BoundedLoadDecision {
    /// Returns the candidate selected after applying the load bound.
    #[must_use]
    pub const fn selection(self) -> Selection {
        self.selection
    }

    /// Returns the unconstrained weighted-rendezvous owner.
    #[must_use]
    pub const fn affinity(self) -> Selection {
        self.affinity
    }

    /// Returns whether the affinity owner was bypassed due to load.
    #[must_use]
    pub const fn spilled(self) -> bool {
        self.selection.index() != self.affinity.index()
    }

    /// Returns the selected candidate's sampled concurrent load.
    #[must_use]
    pub const fn selected_load(self) -> u64 {
        self.selected_load
    }

    /// Returns the selected candidate's capacity for this prospective request.
    ///
    /// This is `u128` so even the largest supported factor and candidate weight
    /// can be reported without saturation.
    #[must_use]
    pub const fn selected_capacity(self) -> u128 {
        self.selected_capacity
    }
}

#[derive(Clone, Copy, Debug)]
struct LoadSample {
    index: usize,
    load: u64,
    weight: u32,
}

#[derive(Clone, Copy, Debug)]
struct Winner {
    index: usize,
    score: f64,
    hash: u64,
    load: u64,
    capacity: u128,
}

/// Weighted key affinity with an explicit prospective-load bound.
///
/// Weighted rendezvous hashing defines a deterministic preference order. The
/// highest-ranked candidate whose sampled concurrent load is below its
/// prospective capacity wins. Capacity is `ceil(factor * (total_load + 1) *
/// weight / (100 * total_weight))`, where `factor` is expressed as a
/// percentage. This preserves ordinary weighted rendezvous behavior while
/// every candidate is below its bound and spills hot keys to a deterministic
/// alternate otherwise.
///
/// Candidate load metrics must produce `u64` counts. [`crate::InFlight`] is the
/// intended tracker; latency-derived [`crate::LoadScore`] values deliberately
/// do not satisfy this policy's contract. Every eligible load is sampled once.
/// The policy retains `O(n)` scratch space so unchanged candidate counts do not
/// allocate after warmup. Selection takes `O(n)` time.
///
/// The bound is computed for the request being selected, not only the current
/// load. Given one coherent snapshot and a balance factor of at least 100,
/// there is always at least one candidate below its prospective capacity.
/// Concurrent selectors can race after sampling, so enforcing a strict process
/// limit still belongs to an atomic admission guard such as
/// [`crate::InFlight::with_limit`].
///
/// Eligible duplicate identities receive the same rendezvous draw and are
/// treated as distinct capacity entries. Callers that require globally stable
/// affinity should provide unique identities.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, policy::BoundedLoadRendezvous};
///
/// let backends = [
///     Backend::new("hot").with_load(100_u64),
///     Backend::new("cool").with_load(0_u64),
/// ];
/// let mut policy = BoundedLoadRendezvous::default();
/// let selected = policy.pick(&backends, &"customer-42")?;
/// assert!(selected.index() < backends.len());
/// # Ok::<(), poise_core::PickError>(())
/// ```
#[derive(Clone, Debug)]
pub struct BoundedLoadRendezvous<S = FnvBuildHasher> {
    config: BoundedLoadConfig,
    hash_builder: S,
    samples: Vec<LoadSample>,
}

impl BoundedLoadRendezvous<FnvBuildHasher> {
    /// Creates a deterministic bounded-load policy.
    #[must_use]
    pub fn new(config: BoundedLoadConfig) -> Self {
        Self::with_hasher(config, FnvBuildHasher::default())
    }
}

impl<S> BoundedLoadRendezvous<S> {
    /// Creates a policy with a caller-provided hash builder.
    #[must_use]
    pub const fn with_hasher(config: BoundedLoadConfig, hash_builder: S) -> Self {
        Self {
            config,
            hash_builder,
            samples: Vec::new(),
        }
    }

    /// Returns the configuration.
    #[must_use]
    pub const fn config(&self) -> BoundedLoadConfig {
        self.config
    }

    /// Returns the hash builder.
    #[must_use]
    pub const fn hash_builder(&self) -> &S {
        &self.hash_builder
    }

    /// Returns retained scratch capacity in eligible-candidate samples.
    #[must_use]
    pub fn scratch_capacity(&self) -> usize {
        self.samples.capacity()
    }

    /// Releases retained sampling scratch space.
    pub fn shrink_to_fit(&mut self) {
        self.samples.shrink_to_fit();
    }

    /// Decomposes the policy into configuration and hash builder.
    #[must_use]
    pub fn into_parts(self) -> (BoundedLoadConfig, S) {
        (self.config, self.hash_builder)
    }
}

impl Default for BoundedLoadRendezvous<FnvBuildHasher> {
    fn default() -> Self {
        Self::new(BoundedLoadConfig::default())
    }
}

impl<S> BoundedLoadRendezvous<S>
where
    S: BuildHasher,
{
    /// Selects a candidate and reports whether load displaced its affinity owner.
    ///
    /// # Errors
    ///
    /// Returns the ordinary empty or ineligible [`PickError`],
    /// [`PickError::WeightOverflow`] if eligible weights cannot be summed,
    /// [`PickError::LoadOverflow`] if load plus the prospective request cannot
    /// be represented, or [`PickError::StateCapacityExceeded`] if sampling
    /// scratch cannot be allocated.
    pub fn decide<C, Context>(
        &mut self,
        candidates: &[C],
        context: &Context,
    ) -> Result<BoundedLoadDecision, PickError>
    where
        C: Candidate,
        C::Id: Hash,
        C::Load: LoadMetric<Metric = u64>,
        Context: Hash + ?Sized,
    {
        if candidates.is_empty() {
            return Err(PickError::Empty);
        }

        let eligible_count = candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
            .count();
        if eligible_count == 0 {
            return Err(PickError::NoEligibleCandidates);
        }

        self.samples.clear();
        self.samples
            .try_reserve(eligible_count)
            .map_err(|_| PickError::StateCapacityExceeded)?;

        let mut total_load = 0_u64;
        let mut total_weight = 0_u64;
        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }
            let load = candidate.load().measure();
            let weight = candidate.weight().get();
            total_load = total_load
                .checked_add(load)
                .ok_or(PickError::LoadOverflow)?;
            total_weight = total_weight
                .checked_add(u64::from(weight))
                .ok_or(PickError::WeightOverflow)?;
            self.samples.push(LoadSample {
                index,
                load,
                weight,
            });
        }

        let prospective_load = total_load.checked_add(1).ok_or(PickError::LoadOverflow)?;
        let factor = self.config.balance_factor_percent.get();
        let mut affinity = None;
        let mut selected = None;

        for sample in &self.samples {
            let candidate = &candidates[sample.index];
            let hash = rendezvous_hash(&self.hash_builder, context, candidate.id());
            let score = exponential_score(hash, sample.weight);
            let capacity = capacity_for(prospective_load, sample.weight, total_weight, factor)?;
            let winner = Winner {
                index: sample.index,
                score,
                hash,
                load: sample.load,
                capacity,
            };

            if affinity.is_none_or(|current| winner_beats(winner, current)) {
                affinity = Some(winner);
            }
            if u128::from(sample.load) < capacity
                && selected.is_none_or(|current| winner_beats(winner, current))
            {
                selected = Some(winner);
            }
        }

        let affinity = affinity.ok_or_else(|| no_candidate_error(candidates.len()))?;
        let Some(selected) = selected else {
            // With factor >= 100, positive weights, and prospective_load equal
            // to current total plus one, the sum of capacities exceeds current
            // load. This branch protects the API if that invariant is changed.
            return Err(PickError::StateCapacityExceeded);
        };
        Ok(BoundedLoadDecision {
            selection: Selection::new(selected.index),
            affinity: Selection::new(affinity.index),
            selected_load: selected.load,
            selected_capacity: selected.capacity,
        })
    }
}

impl<C, Context, S> Policy<C, Context> for BoundedLoadRendezvous<S>
where
    C: Candidate,
    C::Id: Hash,
    C::Load: LoadMetric<Metric = u64>,
    Context: Hash + ?Sized,
    S: BuildHasher,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        self.decide(candidates, context)
            .map(BoundedLoadDecision::selection)
    }
}

fn winner_beats(candidate: Winner, current: Winner) -> bool {
    weighted_score_wins(candidate.score, candidate.hash, current.score, current.hash)
}

fn capacity_for(
    prospective_load: u64,
    weight: u32,
    total_weight: u64,
    factor_percent: u32,
) -> Result<u128, PickError> {
    let numerator = u128::from(prospective_load)
        .checked_mul(u128::from(weight))
        .and_then(|value| value.checked_mul(u128::from(factor_percent)))
        .ok_or(PickError::LoadOverflow)?;
    let denominator = u128::from(total_weight) * 100;
    let quotient = numerator / denominator;
    Ok(quotient + u128::from(numerator % denominator != 0))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{Backend, InFlight, Status, Weight, policy::WeightedRendezvous};

    use super::*;

    fn config(percent: u32) -> BoundedLoadConfig {
        BoundedLoadConfig::new(percent).unwrap()
    }

    fn backend(id: &'static str, weight: u32, load: u64) -> Backend<&'static str, (), u64> {
        Backend::new(id)
            .with_weight(Weight::new(weight).unwrap())
            .with_load(load)
    }

    #[test]
    fn configuration_rejects_factors_below_average() {
        assert_eq!(
            BoundedLoadConfig::new(0),
            Err(BoundedLoadConfigError::BelowAverage)
        );
        assert_eq!(
            BoundedLoadConfig::new(99),
            Err(BoundedLoadConfigError::BelowAverage)
        );
        assert_eq!(config(100).balance_factor_percent().get(), 100);
        assert_eq!(
            BoundedLoadConfig::default().balance_factor_percent().get(),
            DEFAULT_BALANCE_FACTOR_PERCENT
        );
    }

    #[test]
    fn idle_selection_exactly_matches_weighted_rendezvous() {
        let candidates = [backend("a", 1, 0), backend("b", 3, 0), backend("c", 6, 0)];
        let mut bounded = BoundedLoadRendezvous::default();
        let mut affinity = WeightedRendezvous::new();

        for key in 0..20_000_u64 {
            assert_eq!(
                bounded.pick(&candidates, &key),
                affinity.pick(&candidates, &key)
            );
        }
    }

    #[test]
    fn overloaded_owner_spills_to_the_next_affinity_choice() {
        let idle = [backend("a", 1, 0), backend("b", 1, 0), backend("c", 1, 0)];
        let mut base = WeightedRendezvous::new();
        let key = 42_u64;
        let owner = base.pick(&idle, &key).unwrap().index();
        let loaded: [Backend<&str, (), u64>; 3] = std::array::from_fn(|index| {
            backend(["a", "b", "c"][index], 1, u64::from(index == owner) * 100)
        });
        let alternates: [Backend<&str, (), u64>; 3] = std::array::from_fn(|index| {
            if index == owner {
                backend(["a", "b", "c"][index], 1, 0).with_status(Status::Unavailable)
            } else {
                backend(["a", "b", "c"][index], 1, 0)
            }
        });
        let expected = base.pick(&alternates, &key).unwrap();
        let mut policy = BoundedLoadRendezvous::default();

        let decision = policy.decide(&loaded, &key).unwrap();
        assert_eq!(decision.affinity().index(), owner);
        assert_eq!(decision.selection(), expected);
        assert!(decision.spilled());
        assert!(u128::from(decision.selected_load()) < decision.selected_capacity());
    }

    #[test]
    fn every_selection_respects_its_prospective_capacity() {
        let mut policy = BoundedLoadRendezvous::new(config(120));
        for first in 0..8_u64 {
            for second in 0..8_u64 {
                for third in 0..8_u64 {
                    let candidates = [
                        backend("a", 1, first),
                        backend("b", 2, second),
                        backend("c", 3, third),
                    ];
                    let decision = policy.decide(&candidates, &17_u64).unwrap();
                    assert!(u128::from(decision.selected_load()) < decision.selected_capacity());
                }
            }
        }
    }

    #[test]
    fn tight_bound_respects_weighted_capacity() {
        let candidates = [backend("small", 1, 0), backend("large", 3, 9)];
        let mut policy = BoundedLoadRendezvous::new(config(100));

        for key in 0..100_u64 {
            let decision = policy.decide(&candidates, &key).unwrap();
            assert_eq!(decision.selection().index(), 0);
            assert_eq!(decision.selected_capacity(), 3);
        }
    }

    struct CountedLoad {
        value: u64,
        samples: Cell<usize>,
    }

    impl LoadMetric for CountedLoad {
        type Metric = u64;

        fn measure(&self) -> Self::Metric {
            self.samples.set(self.samples.get() + 1);
            self.value
        }
    }

    #[test]
    fn eligible_loads_are_sampled_exactly_once_and_scratch_is_reused() {
        let candidates = [
            Backend::new("a").with_load(CountedLoad {
                value: 1,
                samples: Cell::new(0),
            }),
            Backend::new("b").with_load(CountedLoad {
                value: 2,
                samples: Cell::new(0),
            }),
        ];
        let mut policy = BoundedLoadRendezvous::default();

        policy.decide(&candidates, &0_u64).unwrap();
        let capacity = policy.scratch_capacity();
        policy.decide(&candidates, &1_u64).unwrap();

        assert_eq!(candidates[0].load().samples.get(), 2);
        assert_eq!(candidates[1].load().samples.get(), 2);
        assert_eq!(policy.scratch_capacity(), capacity);
    }

    #[test]
    fn in_flight_is_a_supported_live_counter() {
        let first = InFlight::new();
        let second = InFlight::new();
        let _guards: Vec<_> = (0..10).map(|_| first.start().unwrap()).collect();
        let candidates = [
            Backend::new("busy").with_load(first),
            Backend::new("idle").with_load(second),
        ];
        let mut policy = BoundedLoadRendezvous::new(config(100));

        assert_eq!(policy.pick(&candidates, &0_u64).unwrap().index(), 1);
    }

    #[test]
    fn load_overflow_is_explicit() {
        let candidates = [backend("a", 1, u64::MAX), backend("b", 1, 1)];
        let mut policy = BoundedLoadRendezvous::default();

        assert_eq!(
            policy.pick(&candidates, &0_u64),
            Err(PickError::LoadOverflow)
        );
    }

    #[test]
    fn reordering_preserves_the_winning_identity() {
        let left = [backend("a", 1, 2), backend("b", 2, 1), backend("c", 3, 4)];
        let right = [backend("c", 3, 4), backend("a", 1, 2), backend("b", 2, 1)];
        let mut policy = BoundedLoadRendezvous::default();

        for key in 0..10_000_u64 {
            let left_id = left[policy.pick(&left, &key).unwrap().index()].id();
            let right_id = right[policy.pick(&right, &key).unwrap().index()].id();
            assert_eq!(left_id, right_id);
        }
    }

    #[test]
    fn distinguishes_empty_from_nonempty_ineligible_slices() {
        let empty: [Backend<&str, (), u64>; 0] = [];
        let unavailable = [backend("a", 1, 0).with_status(Status::Unavailable)];
        let mut policy = BoundedLoadRendezvous::default();

        assert_eq!(policy.pick(&empty, &0_u64), Err(PickError::Empty));
        assert_eq!(
            policy.pick(&unavailable, &0_u64),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
