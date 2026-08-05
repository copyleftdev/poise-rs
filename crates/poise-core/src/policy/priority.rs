use std::{error::Error, fmt};

use rand::{Rng, RngExt, SeedableRng, rngs::StdRng};

use crate::{Candidate, PickError, Policy, Selection, Status, Weight};

const TRAFFIC_UNITS: u64 = 1_000_000;

/// Default percentage used to turn available capacity into traffic share.
pub const DEFAULT_OVERPROVISIONING_FACTOR_PERCENT: u32 = 140;

/// Default availability percentage below which an underprovisioned priority panics.
pub const DEFAULT_PANIC_THRESHOLD_PERCENT: u32 = 50;

/// Behavior when a priority enters panic mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PanicMode {
    /// Route across explicitly panic-eligible members, including unhealthy ones.
    #[default]
    UseAll,
    /// Reject traffic assigned to the panicking priority.
    FailClosed,
}

/// Availability and panic policy for [`PriorityWeightedRandom`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PriorityConfig {
    overprovisioning_factor_percent: u32,
    panic_threshold_percent: u32,
    panic_mode: PanicMode,
}

impl PriorityConfig {
    /// Creates a priority-routing configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PriorityConfigError::Underprovisioned`] when the
    /// overprovisioning factor is below 100 percent or
    /// [`PriorityConfigError::InvalidPanicThreshold`] when the panic threshold
    /// exceeds 100 percent.
    pub const fn new(
        overprovisioning_factor_percent: u32,
        panic_threshold_percent: u32,
        panic_mode: PanicMode,
    ) -> Result<Self, PriorityConfigError> {
        if overprovisioning_factor_percent < 100 {
            return Err(PriorityConfigError::Underprovisioned);
        }
        if panic_threshold_percent > 100 {
            return Err(PriorityConfigError::InvalidPanicThreshold);
        }
        Ok(Self {
            overprovisioning_factor_percent,
            panic_threshold_percent,
            panic_mode,
        })
    }

    /// Returns the availability multiplier as a percentage.
    #[must_use]
    pub const fn overprovisioning_factor_percent(self) -> u32 {
        self.overprovisioning_factor_percent
    }

    /// Returns the raw-availability percentage below which panic is considered.
    #[must_use]
    pub const fn panic_threshold_percent(self) -> u32 {
        self.panic_threshold_percent
    }

    /// Returns panic behavior.
    #[must_use]
    pub const fn panic_mode(self) -> PanicMode {
        self.panic_mode
    }
}

impl Default for PriorityConfig {
    fn default() -> Self {
        Self {
            overprovisioning_factor_percent: DEFAULT_OVERPROVISIONING_FACTOR_PERCENT,
            panic_threshold_percent: DEFAULT_PANIC_THRESHOLD_PERCENT,
            panic_mode: PanicMode::UseAll,
        }
    }
}

/// Invalid priority-routing configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PriorityConfigError {
    /// An overprovisioning factor below 100 cannot supply a full healthy share.
    Underprovisioned,
    /// Panic threshold percentages must be at most 100.
    InvalidPanicThreshold,
}

impl fmt::Display for PriorityConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Underprovisioned => {
                formatter.write_str("priority overprovisioning factor must be at least 100 percent")
            }
            Self::InvalidPanicThreshold => {
                formatter.write_str("priority panic threshold must not exceed 100 percent")
            }
        }
    }
}

impl Error for PriorityConfigError {}

/// A candidate that belongs to an ordered failover priority.
pub trait PriorityCandidate: Candidate {
    /// Returns the priority number; lower values receive traffic first.
    fn priority(&self) -> u32;

    /// Returns whether this candidate counts as configured priority capacity.
    ///
    /// Draining candidates are excluded by default.
    fn is_priority_member(&self) -> bool {
        self.status() != Status::Draining
    }

    /// Returns whether panic mode may bypass normal eligibility for this member.
    ///
    /// Implementations should return `false` for hard capacity or policy
    /// exclusions that panic must never override.
    fn is_panic_eligible(&self) -> bool {
        self.is_priority_member()
    }
}

/// Adds priority and panic metadata to an arbitrary [`Candidate`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prioritized<C> {
    inner: C,
    priority: u32,
    allow_in_panic: bool,
}

impl<C> Prioritized<C> {
    /// Wraps a candidate at `priority`, allowing it in use-all panic by default.
    #[must_use]
    pub const fn new(inner: C, priority: u32) -> Self {
        Self {
            inner,
            priority,
            allow_in_panic: true,
        }
    }

    /// Sets whether panic may bypass the inner candidate's normal eligibility.
    #[must_use]
    pub const fn with_panic_eligibility(mut self, allow: bool) -> Self {
        self.allow_in_panic = allow;
        self
    }

    /// Returns the wrapped candidate.
    #[must_use]
    pub const fn inner(&self) -> &C {
        &self.inner
    }

    /// Returns the priority number.
    #[must_use]
    pub const fn priority(&self) -> u32 {
        self.priority
    }

    /// Returns whether this wrapper permits use-all panic selection.
    #[must_use]
    pub const fn allows_panic(&self) -> bool {
        self.allow_in_panic
    }

    /// Returns mutable access to the wrapped candidate.
    pub const fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Removes the priority wrapper.
    #[must_use]
    pub fn into_inner(self) -> C {
        self.inner
    }

    /// Updates the priority number.
    pub const fn set_priority(&mut self, priority: u32) {
        self.priority = priority;
    }
}

impl<C> Candidate for Prioritized<C>
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

impl<C> PriorityCandidate for Prioritized<C>
where
    C: Candidate,
{
    fn priority(&self) -> u32 {
        self.priority
    }

    fn is_panic_eligible(&self) -> bool {
        self.allow_in_panic && self.is_priority_member()
    }
}

/// Eligibility mode used for one priority decision.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PriorityMode {
    /// Only normally eligible members participated.
    Healthy,
    /// Normal eligibility was bypassed for explicitly panic-eligible members.
    Panic,
}

/// A selected endpoint with its priority and eligibility mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PriorityDecision {
    selection: Selection,
    priority: u32,
    mode: PriorityMode,
}

impl PriorityDecision {
    /// Returns the selected candidate index.
    #[must_use]
    pub const fn selection(self) -> Selection {
        self.selection
    }

    /// Returns the chosen priority number.
    #[must_use]
    pub const fn priority(self) -> u32 {
        self.priority
    }

    /// Returns whether normal or panic eligibility was used.
    #[must_use]
    pub const fn mode(self) -> PriorityMode {
        self.mode
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MemberSample {
    pub(super) index: usize,
    pub(super) priority: u32,
    pub(super) weight: u32,
    pub(super) eligible: bool,
    pub(super) panic_eligible: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PriorityScope {
    pub(super) priority: u32,
    pub(super) mode: PriorityMode,
}

#[derive(Clone, Copy, Debug)]
struct PriorityGroup {
    priority: u32,
    total_weight: u64,
    eligible_weight: u64,
    panic_weight: u64,
    raw_availability: u64,
    effective_availability: u64,
    traffic: u64,
    panic: bool,
}

impl PriorityGroup {
    const fn from_member(member: MemberSample) -> Self {
        let weight = member.weight as u64;
        Self {
            priority: member.priority,
            total_weight: weight,
            eligible_weight: if member.eligible { weight } else { 0 },
            panic_weight: if member.panic_eligible { weight } else { 0 },
            raw_availability: 0,
            effective_availability: 0,
            traffic: 0,
            panic: false,
        }
    }
}

/// Weighted random routing across ordered failover priorities.
///
/// Each priority's normally eligible weight is divided by its configured
/// non-draining weight, then multiplied by the overprovisioning factor. Lower
/// priority numbers consume the resulting traffic capacity first; later
/// priorities receive only the shortfall. If combined normalized availability
/// is below 100 percent, shares are normalized across the remaining capacity
/// and priorities below the panic threshold use the configured [`PanicMode`].
///
/// Membership and eligibility are sampled once into retained `O(n)` scratch.
/// Group calculation is `O(n log n)` due to priority sorting; endpoint selection
/// is `O(n)`. Unchanged high-water candidate counts allocate nothing after
/// warmup. Candidate weights affect availability, priority share during global
/// panic, and endpoint selection within a priority.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, Status, policy::{Prioritized, PriorityWeightedRandom}};
///
/// let backends = [
///     Prioritized::new(Backend::new("primary").with_status(Status::Unavailable), 0),
///     Prioritized::new(Backend::new("failover"), 1),
/// ];
/// let mut policy = PriorityWeightedRandom::seeded(7);
/// assert_eq!(policy.pick(&backends, &())?.index(), 1);
/// # Ok::<(), poise_core::PickError>(())
/// ```
#[derive(Clone, Debug)]
pub struct PriorityWeightedRandom<R = StdRng> {
    config: PriorityConfig,
    rng: R,
    members: Vec<MemberSample>,
    groups: Vec<PriorityGroup>,
}

impl PriorityWeightedRandom<StdRng> {
    /// Creates a policy seeded from the process random-number source.
    #[must_use]
    pub fn new(config: PriorityConfig) -> Self {
        Self::with_rng(config, StdRng::from_rng(&mut rand::rng()))
    }

    /// Creates a reproducible policy using the default configuration.
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

impl<R> PriorityWeightedRandom<R> {
    /// Creates a policy with a caller-provided random-number generator.
    #[must_use]
    pub const fn with_rng(config: PriorityConfig, rng: R) -> Self {
        Self {
            config,
            rng,
            members: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// Returns the configuration.
    #[must_use]
    pub const fn config(&self) -> PriorityConfig {
        self.config
    }

    /// Returns the random-number generator.
    #[must_use]
    pub const fn rng(&self) -> &R {
        &self.rng
    }

    /// Returns mutable access to the random-number generator.
    pub const fn rng_mut(&mut self) -> &mut R {
        &mut self.rng
    }

    /// Returns retained member/group scratch capacities.
    #[must_use]
    pub fn scratch_capacity(&self) -> (usize, usize) {
        (self.members.capacity(), self.groups.capacity())
    }

    /// Releases retained calculation scratch space.
    pub fn shrink_to_fit(&mut self) {
        self.members.shrink_to_fit();
        self.groups.shrink_to_fit();
    }

    /// Decomposes the policy into configuration and random-number generator.
    #[must_use]
    pub fn into_parts(self) -> (PriorityConfig, R) {
        (self.config, self.rng)
    }
}

impl Default for PriorityWeightedRandom<StdRng> {
    fn default() -> Self {
        Self::new(PriorityConfig::default())
    }
}

impl<R> PriorityWeightedRandom<R>
where
    R: Rng,
{
    /// Selects an endpoint and reports its priority and panic state.
    ///
    /// # Errors
    ///
    /// Returns the standard empty or ineligible [`PickError`],
    /// [`PickError::WeightOverflow`] when sampled weights cannot be accumulated,
    /// [`PickError::StateCapacityExceeded`] when scratch growth fails, or
    /// [`PickError::PanicRejected`] when fail-closed panic receives traffic.
    pub fn decide<C>(&mut self, candidates: &[C]) -> Result<PriorityDecision, PickError>
    where
        C: PriorityCandidate,
    {
        let scope = self.select_scope(candidates)?;
        let selection = self.select_member(scope)?;
        Ok(PriorityDecision {
            selection,
            priority: scope.priority,
            mode: scope.mode,
        })
    }

    pub(super) fn select_scope<C>(&mut self, candidates: &[C]) -> Result<PriorityScope, PickError>
    where
        C: PriorityCandidate,
    {
        self.sample(candidates)?;
        self.calculate_groups()?;

        let total_effective = self.groups.iter().fold(0_u64, |total, group| {
            total
                .saturating_add(group.effective_availability)
                .min(TRAFFIC_UNITS)
        });
        let shortage = total_effective < TRAFFIC_UNITS;
        let panic_threshold = u64::from(self.config.panic_threshold_percent) * TRAFFIC_UNITS / 100;
        for group in &mut self.groups {
            group.panic =
                shortage && panic_threshold > 0 && group.raw_availability < panic_threshold;
            group.traffic = 0;
        }

        if total_effective == 0 {
            return self.select_global_panic_scope();
        }

        if shortage {
            for group in &mut self.groups {
                group.traffic = group.effective_availability;
            }
        } else {
            let mut remaining = TRAFFIC_UNITS;
            for group in &mut self.groups {
                group.traffic = group.effective_availability.min(remaining);
                remaining -= group.traffic;
            }
        }

        let total_traffic = if shortage {
            self.groups.iter().map(|group| group.traffic).sum()
        } else {
            TRAFFIC_UNITS
        };
        let group_index = choose_group(&mut self.rng, &self.groups, total_traffic)
            .ok_or(PickError::NoEligibleCandidates)?;
        self.scope_from_group(group_index)
    }

    pub(super) fn member_samples(&self) -> &[MemberSample] {
        &self.members
    }

    pub(super) fn random_below(&mut self, upper: u64) -> u64 {
        self.rng.random_range(0..upper)
    }

    fn sample<C>(&mut self, candidates: &[C]) -> Result<(), PickError>
    where
        C: PriorityCandidate,
    {
        if candidates.is_empty() {
            return Err(PickError::Empty);
        }
        self.members.clear();
        self.members
            .try_reserve(candidates.len())
            .map_err(|_| PickError::StateCapacityExceeded)?;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_priority_member() {
                continue;
            }
            self.members.push(MemberSample {
                index,
                priority: candidate.priority(),
                weight: candidate.weight().get(),
                eligible: candidate.is_eligible(),
                panic_eligible: candidate.is_panic_eligible(),
            });
        }
        if self.members.is_empty() {
            return Err(PickError::NoEligibleCandidates);
        }
        Ok(())
    }

    fn calculate_groups(&mut self) -> Result<(), PickError> {
        self.groups.clear();
        self.groups
            .try_reserve(self.members.len())
            .map_err(|_| PickError::StateCapacityExceeded)?;
        self.groups
            .extend(self.members.iter().copied().map(PriorityGroup::from_member));
        self.groups.sort_unstable_by_key(|group| group.priority);

        let mut write = 0_usize;
        for read in 0..self.groups.len() {
            let group = self.groups[read];
            if write > 0 && self.groups[write - 1].priority == group.priority {
                let current = &mut self.groups[write - 1];
                current.total_weight = current
                    .total_weight
                    .checked_add(group.total_weight)
                    .ok_or(PickError::WeightOverflow)?;
                current.eligible_weight = current
                    .eligible_weight
                    .checked_add(group.eligible_weight)
                    .ok_or(PickError::WeightOverflow)?;
                current.panic_weight = current
                    .panic_weight
                    .checked_add(group.panic_weight)
                    .ok_or(PickError::WeightOverflow)?;
            } else {
                self.groups[write] = group;
                write += 1;
            }
        }
        self.groups.truncate(write);

        for group in &mut self.groups {
            group.raw_availability = ratio_units(group.eligible_weight, group.total_weight, 100);
            group.effective_availability = ratio_units(
                group.eligible_weight,
                group.total_weight,
                self.config.overprovisioning_factor_percent,
            );
        }
        Ok(())
    }

    fn select_global_panic_scope(&mut self) -> Result<PriorityScope, PickError> {
        let panic_weight =
            self.groups
                .iter()
                .filter(|group| group.panic)
                .try_fold(0_u64, |total, group| {
                    total
                        .checked_add(group.panic_weight)
                        .ok_or(PickError::WeightOverflow)
                })?;
        if panic_weight == 0 {
            return Err(PickError::NoEligibleCandidates);
        }
        if self.config.panic_mode == PanicMode::FailClosed {
            return Err(PickError::PanicRejected);
        }

        let mut ticket = self.rng.random_range(0..panic_weight);
        let group_index = self
            .groups
            .iter()
            .position(|group| {
                if !group.panic {
                    return false;
                }
                if ticket < group.panic_weight {
                    true
                } else {
                    ticket -= group.panic_weight;
                    false
                }
            })
            .ok_or(PickError::NoEligibleCandidates)?;
        self.scope_from_group(group_index)
    }

    fn scope_from_group(&self, group_index: usize) -> Result<PriorityScope, PickError> {
        let group = self.groups[group_index];
        if group.panic && self.config.panic_mode == PanicMode::FailClosed {
            return Err(PickError::PanicRejected);
        }
        let mode = if group.panic {
            PriorityMode::Panic
        } else {
            PriorityMode::Healthy
        };
        let selected_weight = if group.panic {
            group.panic_weight
        } else {
            group.eligible_weight
        };
        if selected_weight == 0 {
            return Err(PickError::NoEligibleCandidates);
        }

        Ok(PriorityScope {
            priority: group.priority,
            mode,
        })
    }

    fn select_member(&mut self, scope: PriorityScope) -> Result<Selection, PickError> {
        let total_weight = self.members.iter().try_fold(0_u64, |total, member| {
            if member_in_scope(*member, scope) {
                total
                    .checked_add(u64::from(member.weight))
                    .ok_or(PickError::WeightOverflow)
            } else {
                Ok(total)
            }
        })?;
        if total_weight == 0 {
            return Err(PickError::NoEligibleCandidates);
        }
        let mut ticket = self.rng.random_range(0..total_weight);
        let selected = self
            .members
            .iter()
            .find(|member| {
                if !member_in_scope(**member, scope) {
                    return false;
                }
                let weight = u64::from(member.weight);
                if ticket < weight {
                    true
                } else {
                    ticket -= weight;
                    false
                }
            })
            .ok_or(PickError::NoEligibleCandidates)?;
        Ok(Selection::new(selected.index))
    }
}

impl<C, Context, R> Policy<C, Context> for PriorityWeightedRandom<R>
where
    C: PriorityCandidate,
    Context: ?Sized,
    R: Rng,
{
    fn pick(&mut self, candidates: &[C], _context: &Context) -> Result<Selection, PickError> {
        self.decide(candidates).map(PriorityDecision::selection)
    }
}

pub(super) fn ratio_units(available: u64, total: u64, factor_percent: u32) -> u64 {
    let numerator = u128::from(available) * u128::from(factor_percent) * u128::from(TRAFFIC_UNITS);
    let denominator = u128::from(total) * 100;
    u64::try_from((numerator / denominator).min(u128::from(TRAFFIC_UNITS)))
        .expect("a capped traffic share fits u64")
}

pub(super) const fn member_in_scope(member: MemberSample, scope: PriorityScope) -> bool {
    member.priority == scope.priority
        && match scope.mode {
            PriorityMode::Healthy => member.eligible,
            PriorityMode::Panic => member.panic_eligible,
        }
}

fn choose_group<R>(rng: &mut R, groups: &[PriorityGroup], total: u64) -> Option<usize>
where
    R: Rng,
{
    if total == 0 {
        return None;
    }
    let mut ticket = rng.random_range(0..total);
    groups.iter().position(|group| {
        if ticket < group.traffic {
            true
        } else {
            ticket -= group.traffic;
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::Backend;

    use super::*;

    fn config(overprovisioning: u32, panic: u32, mode: PanicMode) -> PriorityConfig {
        PriorityConfig::new(overprovisioning, panic, mode).unwrap()
    }

    fn backend(
        id: &'static str,
        priority: u32,
        status: Status,
        weight: u32,
    ) -> Prioritized<Backend<&'static str>> {
        Prioritized::new(
            Backend::new(id)
                .with_status(status)
                .with_weight(Weight::new(weight).unwrap()),
            priority,
        )
    }

    #[test]
    fn configuration_validates_percentages() {
        assert_eq!(
            PriorityConfig::new(99, 50, PanicMode::UseAll),
            Err(PriorityConfigError::Underprovisioned)
        );
        assert_eq!(
            PriorityConfig::new(140, 101, PanicMode::UseAll),
            Err(PriorityConfigError::InvalidPanicThreshold)
        );
        assert_eq!(
            PriorityConfig::default().overprovisioning_factor_percent(),
            140
        );
        assert_eq!(PriorityConfig::default().panic_threshold_percent(), 50);
    }

    #[test]
    fn fully_available_primary_receives_all_traffic() {
        let candidates = [
            backend("primary-a", 0, Status::Ready, 1),
            backend("primary-b", 0, Status::Ready, 1),
            backend("failover", 1, Status::Ready, 1),
        ];
        let mut policy = PriorityWeightedRandom::seeded(1);

        for _ in 0..1_000 {
            assert_eq!(policy.decide(&candidates).unwrap().priority(), 0);
        }
    }

    #[test]
    fn lower_priority_receives_only_the_capacity_shortfall() {
        let candidates = [
            backend("p0-ready", 0, Status::Ready, 1),
            backend("p0-down", 0, Status::Unavailable, 1),
            backend("p1-ready", 1, Status::Ready, 2),
        ];
        let mut policy = PriorityWeightedRandom::seeded_with(config(100, 50, PanicMode::UseAll), 2);
        let mut primary = 0_u32;

        for _ in 0..20_000 {
            primary += u32::from(policy.decide(&candidates).unwrap().priority() == 0);
        }
        assert!(
            (9_700..=10_300).contains(&primary),
            "primary count {primary}"
        );
    }

    #[test]
    fn overprovisioning_absorbs_documented_primary_headroom() {
        let candidates = [
            backend("ready-a", 0, Status::Ready, 1),
            backend("ready-b", 0, Status::Ready, 1),
            backend("ready-c", 0, Status::Ready, 1),
            backend("ready-d", 0, Status::Ready, 1),
            backend("ready-e", 0, Status::Ready, 1),
            backend("down-a", 0, Status::Unavailable, 1),
            backend("down-b", 0, Status::Unavailable, 1),
            backend("failover", 1, Status::Ready, 1),
        ];
        let mut policy = PriorityWeightedRandom::seeded(11);

        for _ in 0..1_000 {
            assert_eq!(policy.decide(&candidates).unwrap().priority(), 0);
        }
    }

    #[test]
    fn lower_priorities_suppress_panic_when_combined_capacity_is_full() {
        let candidates = [
            backend("p0-ready", 0, Status::Ready, 1),
            backend("p0-down", 0, Status::Unavailable, 3),
            backend("p1-ready", 1, Status::Ready, 4),
        ];
        let mut policy =
            PriorityWeightedRandom::seeded_with(config(100, 50, PanicMode::FailClosed), 3);

        for _ in 0..1_000 {
            let decision = policy.decide(&candidates).unwrap();
            assert_eq!(decision.mode(), PriorityMode::Healthy);
        }
    }

    #[test]
    fn use_all_panic_can_route_to_unhealthy_members() {
        let candidates = [
            backend("ready", 0, Status::Ready, 1),
            backend("down-a", 0, Status::Unavailable, 1),
            backend("down-b", 0, Status::Unavailable, 1),
            backend("down-c", 0, Status::Unavailable, 1),
        ];
        let mut policy = PriorityWeightedRandom::seeded_with(config(100, 50, PanicMode::UseAll), 4);
        let mut unhealthy_selected = false;

        for _ in 0..100 {
            let decision = policy.decide(&candidates).unwrap();
            assert_eq!(decision.mode(), PriorityMode::Panic);
            unhealthy_selected |= decision.selection().index() != 0;
        }
        assert!(unhealthy_selected);
    }

    #[test]
    fn fail_closed_panic_is_an_explicit_error() {
        let candidates = [
            backend("ready", 0, Status::Ready, 1),
            backend("down-a", 0, Status::Unavailable, 1),
            backend("down-b", 0, Status::Unavailable, 1),
        ];
        let mut policy =
            PriorityWeightedRandom::seeded_with(config(100, 50, PanicMode::FailClosed), 5);

        assert_eq!(policy.decide(&candidates), Err(PickError::PanicRejected));
    }

    #[test]
    fn zero_threshold_disables_panic() {
        let candidates = [backend("down", 0, Status::Unavailable, 1)];
        let mut policy = PriorityWeightedRandom::seeded_with(config(100, 0, PanicMode::UseAll), 6);

        assert_eq!(
            policy.decide(&candidates),
            Err(PickError::NoEligibleCandidates)
        );
    }

    #[test]
    fn draining_and_opted_out_members_are_never_revived() {
        let candidates = [
            backend("draining", 0, Status::Draining, 100),
            backend("excluded", 0, Status::Unavailable, 100).with_panic_eligibility(false),
            backend("panic-safe", 0, Status::Unavailable, 1),
        ];
        let mut policy = PriorityWeightedRandom::seeded_with(config(100, 50, PanicMode::UseAll), 7);

        for _ in 0..100 {
            assert_eq!(policy.pick(&candidates, &()).unwrap().index(), 2);
        }
    }

    #[test]
    fn endpoint_weights_are_respected_within_a_priority() {
        let candidates = [
            backend("small", 0, Status::Ready, 1),
            backend("large", 0, Status::Ready, 3),
        ];
        let mut policy = PriorityWeightedRandom::seeded(8);
        let mut large = 0_u32;

        for _ in 0..20_000 {
            large += u32::from(policy.pick(&candidates, &()).unwrap().index() == 1);
        }
        assert!((14_600..=15_400).contains(&large), "large count {large}");
    }

    #[test]
    fn seeded_policies_replay_and_reuse_scratch() {
        let candidates = [
            backend("a", 0, Status::Ready, 1),
            backend("b", 1, Status::Ready, 1),
        ];
        let mut left = PriorityWeightedRandom::seeded(9);
        let mut right = PriorityWeightedRandom::seeded(9);

        for _ in 0..100 {
            assert_eq!(left.decide(&candidates), right.decide(&candidates));
        }
        let capacity = left.scratch_capacity();
        left.decide(&candidates).unwrap();
        assert_eq!(left.scratch_capacity(), capacity);
    }

    #[test]
    fn distinguishes_empty_from_no_priority_members() {
        let empty: [Prioritized<Backend<&str>>; 0] = [];
        let draining = [backend("a", 0, Status::Draining, 1)];
        let mut policy = PriorityWeightedRandom::seeded(10);

        assert_eq!(policy.pick(&empty, &()), Err(PickError::Empty));
        assert_eq!(
            policy.pick(&draining, &()),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
