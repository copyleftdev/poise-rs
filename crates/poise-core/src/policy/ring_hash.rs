use std::{
    collections::HashSet,
    error::Error,
    fmt,
    hash::{BuildHasher, Hash, Hasher},
    num::{NonZeroU32, NonZeroUsize},
};

use crate::{Candidate, FnvBuildHasher, PickError, Policy, Selection, mix64};

use super::no_candidate_error;

/// Memory and distribution policy for [`RingHash`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RingHashConfig {
    virtual_nodes_per_weight: NonZeroU32,
    max_virtual_nodes: NonZeroUsize,
}

impl RingHashConfig {
    /// Creates a bounded virtual-node configuration.
    ///
    /// Candidate weights are divided by the eligible set's greatest common
    /// divisor, then multiplied by `virtual_nodes_per_weight`. Reconciliation
    /// fails before mutation if their total exceeds `max_virtual_nodes`.
    ///
    /// # Errors
    ///
    /// Returns [`RingHashConfigError`] when a unit-weight candidate cannot fit.
    pub fn new(
        virtual_nodes_per_weight: NonZeroU32,
        max_virtual_nodes: NonZeroUsize,
    ) -> Result<Self, RingHashConfigError> {
        let minimum = u64::from(virtual_nodes_per_weight.get());
        let maximum = u64::try_from(max_virtual_nodes.get()).unwrap_or(u64::MAX);
        if minimum > maximum {
            return Err(RingHashConfigError::UnitWeightExceedsCapacity);
        }
        Ok(Self {
            virtual_nodes_per_weight,
            max_virtual_nodes,
        })
    }

    /// Returns virtual nodes assigned per unit of candidate weight.
    #[must_use]
    pub const fn virtual_nodes_per_weight(self) -> NonZeroU32 {
        self.virtual_nodes_per_weight
    }

    /// Returns the hard table-point limit.
    #[must_use]
    pub const fn max_virtual_nodes(self) -> NonZeroUsize {
        self.max_virtual_nodes
    }
}

impl Default for RingHashConfig {
    fn default() -> Self {
        Self {
            virtual_nodes_per_weight: NonZeroU32::new(128).expect("128 is non-zero"),
            max_virtual_nodes: NonZeroUsize::new(1_048_576).expect("the limit is non-zero"),
        }
    }
}

/// Invalid ring-hash configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RingHashConfigError {
    /// The cap cannot hold the points for one unit-weight candidate.
    UnitWeightExceedsCapacity,
}

impl fmt::Display for RingHashConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnitWeightExceedsCapacity => {
                formatter.write_str("max virtual nodes must fit one unit-weight candidate")
            }
        }
    }
}

impl Error for RingHashConfigError {}

/// Result of reconciling a candidate slice with a cached ring.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RingUpdate {
    /// Identity, weight, eligibility, and order matched the cached state.
    Unchanged,
    /// A new table was committed.
    Rebuilt {
        /// Number of eligible candidate identities.
        members: usize,
        /// Number of virtual points in the table.
        virtual_nodes: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Member<Key> {
    key: Key,
    index: usize,
    weight: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Point {
    position: u64,
    owner_hash: u64,
    replica: u64,
    index: usize,
}

/// Weighted consistent hashing over a cached virtual-node ring.
///
/// Eligible candidates receive a number of ring points proportional to their
/// positive capacity weight. Requests hash to the first point clockwise, with
/// wraparound at the end of the `u64` space. Ring construction is `O(r log r)`
/// time and `O(r)` memory for `r` virtual nodes. Each safe [`Policy::pick`]
/// performs an `O(n)` membership validation followed by `O(log r)` lookup; an
/// unchanged lookup allocates nothing and does not rebuild the ring.
///
/// Rebuilds are staged. Duplicate eligible identities, point-count overflow,
/// the configured capacity limit, or allocation failure returns a [`PickError`]
/// without replacing the previous table.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, policy::RingHash};
///
/// let backends = [Backend::new("west"), Backend::new("east")];
/// let mut policy = RingHash::<&str>::default();
/// let first = policy.pick(&backends, &"account-42").unwrap();
/// let replay = policy.pick(&backends, &"account-42").unwrap();
/// assert_eq!(first, replay);
/// assert_eq!(policy.generation(), 1);
/// ```
#[derive(Clone, Debug)]
pub struct RingHash<Key, S = FnvBuildHasher> {
    config: RingHashConfig,
    hash_builder: S,
    members: Vec<Member<Key>>,
    points: Vec<Point>,
    generation: u64,
}

impl<Key> RingHash<Key, FnvBuildHasher> {
    /// Creates an empty deterministic ring.
    #[must_use]
    pub fn new(config: RingHashConfig) -> Self {
        Self::with_hasher(config, FnvBuildHasher::default())
    }
}

impl<Key> Default for RingHash<Key, FnvBuildHasher> {
    fn default() -> Self {
        Self::new(RingHashConfig::default())
    }
}

impl<Key, S> RingHash<Key, S> {
    /// Creates an empty ring with a caller-provided hash builder.
    #[must_use]
    pub const fn with_hasher(config: RingHashConfig, hash_builder: S) -> Self {
        Self {
            config,
            hash_builder,
            members: Vec::new(),
            points: Vec::new(),
            generation: 0,
        }
    }

    /// Returns the configuration.
    #[must_use]
    pub const fn config(&self) -> RingHashConfig {
        self.config
    }

    /// Returns the hash builder.
    #[must_use]
    pub const fn hash_builder(&self) -> &S {
        &self.hash_builder
    }

    /// Returns the committed virtual-node count.
    #[must_use]
    pub fn virtual_node_count(&self) -> usize {
        self.points.len()
    }

    /// Returns the committed eligible-member count.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Returns the number of committed rebuilds, wrapping at `u64::MAX`.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Clears the cached membership and ring.
    pub fn reset(&mut self) {
        self.members.clear();
        self.points.clear();
        self.generation = 0;
    }

    /// Decomposes the policy into configuration and hash builder.
    #[must_use]
    pub fn into_parts(self) -> (RingHashConfig, S) {
        (self.config, self.hash_builder)
    }
}

impl<Key, S> RingHash<Key, S>
where
    Key: Clone + Eq + Hash,
    S: BuildHasher,
{
    /// Rebuilds the ring if identity, slice index, weight, or eligibility changed.
    ///
    /// # Errors
    ///
    /// Returns [`PickError::DuplicateIdentity`] for duplicate eligible keys or
    /// [`PickError::StateCapacityExceeded`] if the configured bound, integer
    /// representation, or allocator cannot hold the staged table.
    pub fn reconcile<C>(&mut self, candidates: &[C]) -> Result<RingUpdate, PickError>
    where
        C: Candidate<Id = Key>,
    {
        if self.matches(candidates) {
            return Ok(RingUpdate::Unchanged);
        }

        let eligible_count = candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
            .count();
        let mut seen = HashSet::new();
        seen.try_reserve(eligible_count)
            .map_err(|_| PickError::StateCapacityExceeded)?;
        let mut weight_divisor = 0_u32;
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
        {
            if !seen.insert(candidate.id()) {
                return Err(PickError::DuplicateIdentity);
            }
            weight_divisor = greatest_common_divisor(weight_divisor, candidate.weight().get());
        }

        let mut total = 0_usize;
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
        {
            let points = points_for(
                candidate.weight().get() / weight_divisor,
                self.config.virtual_nodes_per_weight,
            )?;
            total = total
                .checked_add(points)
                .ok_or(PickError::StateCapacityExceeded)?;
            if total > self.config.max_virtual_nodes.get() {
                return Err(PickError::StateCapacityExceeded);
            }
        }

        let mut next_members = Vec::new();
        next_members
            .try_reserve_exact(seen.len())
            .map_err(|_| PickError::StateCapacityExceeded)?;
        let mut next_points = Vec::new();
        next_points
            .try_reserve_exact(total)
            .map_err(|_| PickError::StateCapacityExceeded)?;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }
            let weight = candidate.weight().get();
            let replicas = points_for(
                weight / weight_divisor,
                self.config.virtual_nodes_per_weight,
            )?;
            let owner_hash = hash_identity(&self.hash_builder, candidate.id());
            next_members.push(Member {
                key: candidate.id().clone(),
                index,
                weight,
            });
            for replica in 0..replicas {
                let replica =
                    u64::try_from(replica).map_err(|_| PickError::StateCapacityExceeded)?;
                next_points.push(Point {
                    position: hash_point(&self.hash_builder, candidate.id(), replica),
                    owner_hash,
                    replica,
                    index,
                });
            }
        }

        next_points.sort_unstable_by_key(|point| {
            (point.position, point.owner_hash, point.replica, point.index)
        });
        self.members = next_members;
        self.points = next_points;
        self.generation = self.generation.wrapping_add(1);
        Ok(RingUpdate::Rebuilt {
            members: self.members.len(),
            virtual_nodes: self.points.len(),
        })
    }

    fn matches<C>(&self, candidates: &[C]) -> bool
    where
        C: Candidate<Id = Key>,
    {
        let mut members = self.members.iter();
        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }
            let Some(member) = members.next() else {
                return false;
            };
            if member.index != index
                || member.weight != candidate.weight().get()
                || member.key != *candidate.id()
            {
                return false;
            }
        }
        members.next().is_none()
    }
}

impl<C, Key, Context, S> Policy<C, Context> for RingHash<Key, S>
where
    C: Candidate<Id = Key>,
    Key: Clone + Eq + Hash,
    Context: Hash + ?Sized,
    S: BuildHasher,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        self.reconcile(candidates)?;
        if self.points.is_empty() {
            return Err(no_candidate_error(candidates.len()));
        }

        let request = hash_request(&self.hash_builder, context);
        let point = self
            .points
            .partition_point(|point| point.position < request);
        let index = self.points.get(point).unwrap_or(&self.points[0]).index;
        Ok(Selection::new(index))
    }
}

fn points_for(weight: u32, replicas: NonZeroU32) -> Result<usize, PickError> {
    let points = u64::from(weight) * u64::from(replicas.get());
    usize::try_from(points).map_err(|_| PickError::StateCapacityExceeded)
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn hash_identity<S, Key>(hash_builder: &S, key: &Key) -> u64
where
    S: BuildHasher,
    Key: Hash + ?Sized,
{
    let mut hasher = hash_builder.build_hasher();
    hasher.write_u8(2);
    key.hash(&mut hasher);
    mix64(hasher.finish())
}

fn hash_point<S, Key>(hash_builder: &S, key: &Key, replica: u64) -> u64
where
    S: BuildHasher,
    Key: Hash + ?Sized,
{
    let mut hasher = hash_builder.build_hasher();
    hasher.write_u8(3);
    key.hash(&mut hasher);
    hasher.write_u8(4);
    replica.hash(&mut hasher);
    mix64(hasher.finish())
}

fn hash_request<S, Context>(hash_builder: &S, context: &Context) -> u64
where
    S: BuildHasher,
    Context: Hash + ?Sized,
{
    let mut hasher = hash_builder.build_hasher();
    hasher.write_u8(5);
    context.hash(&mut hasher);
    mix64(hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::hash::{BuildHasherDefault, Hasher};

    use crate::{Backend, Status, Weight};

    use super::*;

    fn config(replicas: u32, maximum: usize) -> RingHashConfig {
        RingHashConfig::new(
            NonZeroU32::new(replicas).unwrap(),
            NonZeroUsize::new(maximum).unwrap(),
        )
        .unwrap()
    }

    fn backend(id: &'static str, weight: u32) -> Backend<&'static str> {
        Backend::new(id).with_weight(Weight::new(weight).unwrap())
    }

    #[test]
    fn configuration_requires_room_for_one_unit_weight() {
        assert_eq!(
            RingHashConfig::new(NonZeroU32::new(2).unwrap(), NonZeroUsize::new(1).unwrap()),
            Err(RingHashConfigError::UnitWeightExceedsCapacity)
        );
    }

    #[test]
    fn weighted_ring_is_cached_until_membership_changes() {
        let candidates = [backend("a", 1), backend("b", 3)];
        let mut policy = RingHash::new(config(16, 1_000));

        assert_eq!(
            policy.reconcile(&candidates),
            Ok(RingUpdate::Rebuilt {
                members: 2,
                virtual_nodes: 64
            })
        );
        assert_eq!(policy.member_count(), 2);
        assert_eq!(policy.virtual_node_count(), 64);
        assert_eq!(policy.generation(), 1);
        assert_eq!(policy.reconcile(&candidates), Ok(RingUpdate::Unchanged));
        assert_eq!(policy.generation(), 1);

        for key in 0..100_u64 {
            policy.pick(&candidates, &key).unwrap();
        }
        assert_eq!(policy.generation(), 1);
    }

    #[test]
    fn changing_one_weight_invalidates_the_cached_membership() {
        let initial = [backend("a", 1), backend("b", 1)];
        let changed = [backend("a", 1), backend("b", 3)];
        let mut policy = RingHash::new(config(4, 100));

        policy.reconcile(&initial).unwrap();
        assert_eq!(policy.virtual_node_count(), 8);
        policy.reconcile(&changed).unwrap();
        assert_eq!(policy.generation(), 2);
        assert_eq!(policy.virtual_node_count(), 16);
    }

    #[test]
    fn exact_point_hash_selects_that_point_instead_of_its_successor() {
        let candidates = [backend("a", 1), backend("b", 1)];
        let mut policy = RingHash::new(config(1, 10));
        policy.members = vec![
            Member {
                key: "a",
                index: 0,
                weight: 1,
            },
            Member {
                key: "b",
                index: 1,
                weight: 1,
            },
        ];
        let request = hash_request(&policy.hash_builder, &91_u64);
        assert_ne!(request, u64::MAX);
        policy.points = vec![
            Point {
                position: request,
                owner_hash: 0,
                replica: 0,
                index: 0,
            },
            Point {
                position: request + 1,
                owner_hash: 0,
                replica: 0,
                index: 1,
            },
        ];

        assert_eq!(policy.pick(&candidates, &91_u64).unwrap().index(), 0);
    }

    #[test]
    fn identity_hash_is_domain_separated_and_nontrivial() {
        let builder = FnvBuildHasher::default();
        let first = hash_identity(&builder, &1_u64);
        let second = hash_identity(&builder, &2_u64);
        assert_ne!(first, second);
        assert!(![0, 1].contains(&first));
        assert!(![0, 1].contains(&second));
    }

    #[test]
    fn keyspace_distribution_tracks_weights_with_sufficient_points() {
        const KEYS: u32 = 200_000;
        let candidates = [backend("a", 1), backend("b", 3), backend("c", 6)];
        let mut policy = RingHash::new(config(512, 10_000));
        let mut counts = [0_u32; 3];

        for key in 0..KEYS {
            counts[policy.pick(&candidates, &key).unwrap().index()] += 1;
        }

        for (count, expected) in counts.into_iter().zip([0.1_f64, 0.3, 0.6]) {
            let actual = f64::from(count) / f64::from(KEYS);
            assert!(
                (actual - expected).abs() < 0.025,
                "expected {expected:.3}, observed {actual:.3}"
            );
        }
    }

    #[test]
    fn equivalent_weight_ratios_build_identical_rings() {
        let reduced = [backend("a", 1), backend("b", 3)];
        let scaled = [backend("a", 100), backend("b", 300)];
        let mut reduced_policy = RingHash::new(config(16, 1_000));
        let mut scaled_policy = RingHash::new(config(16, 1_000));

        for key in 0..10_000_u64 {
            let reduced_id = reduced[reduced_policy.pick(&reduced, &key).unwrap().index()].id();
            let scaled_id = scaled[scaled_policy.pick(&scaled, &key).unwrap().index()].id();
            assert_eq!(reduced_id, scaled_id);
        }
        assert_eq!(reduced_policy.virtual_node_count(), 64);
        assert_eq!(scaled_policy.virtual_node_count(), 64);
    }

    #[test]
    fn stable_normalization_makes_addition_and_removal_minimally_disruptive() {
        let without_c = [backend("a", 1), backend("b", 3)];
        let with_c = [backend("a", 1), backend("b", 3), backend("c", 2)];
        let mut small = RingHash::new(config(128, 10_000));
        let mut large = RingHash::new(config(128, 10_000));

        for key in 0..50_000_u64 {
            let old_id = without_c[small.pick(&without_c, &key).unwrap().index()].id();
            let new_id = with_c[large.pick(&with_c, &key).unwrap().index()].id();
            if new_id != &"c" {
                assert_eq!(old_id, new_id, "key {key} moved between stable peers");
            }
        }
    }

    #[test]
    fn reordering_rebuilds_indices_without_changing_winning_identity() {
        let left = [backend("a", 1), backend("b", 3), backend("c", 2)];
        let right = [backend("c", 2), backend("a", 1), backend("b", 3)];
        let mut left_policy = RingHash::new(config(128, 10_000));
        let mut right_policy = RingHash::new(config(128, 10_000));

        for key in 0..20_000_u64 {
            let left_id = left[left_policy.pick(&left, &key).unwrap().index()].id();
            let right_id = right[right_policy.pick(&right, &key).unwrap().index()].id();
            assert_eq!(left_id, right_id);
        }
    }

    #[test]
    fn eligibility_changes_rebuild_and_exclude_the_candidate() {
        let ready = [backend("a", 1), backend("b", 1)];
        let unavailable = [
            backend("a", 1).with_status(Status::Unavailable),
            backend("b", 1),
        ];
        let mut policy = RingHash::new(config(16, 1_000));

        policy.pick(&ready, &0_u64).unwrap();
        assert_eq!(policy.generation(), 1);
        for key in 0..100_u64 {
            assert_eq!(policy.pick(&unavailable, &key).unwrap().index(), 1);
        }
        assert_eq!(policy.generation(), 2);
    }

    #[test]
    fn duplicate_and_capacity_failures_do_not_replace_the_live_ring() {
        let good = [backend("a", 1), backend("b", 1)];
        let duplicate = [backend("same", 1), backend("same", 1)];
        let oversized = [backend("large", 3), backend("small", 1)];
        let mut policy = RingHash::new(config(2, 4));

        policy.pick(&good, &0_u64).unwrap();
        assert_eq!(policy.generation(), 1);
        assert_eq!(policy.virtual_node_count(), 4);

        assert_eq!(
            policy.pick(&duplicate, &0_u64),
            Err(PickError::DuplicateIdentity)
        );
        assert_eq!(
            policy.pick(&oversized, &0_u64),
            Err(PickError::StateCapacityExceeded)
        );
        assert_eq!(policy.generation(), 1);
        assert_eq!(policy.virtual_node_count(), 4);
    }

    #[derive(Default)]
    struct ConstantHasher;

    impl Hasher for ConstantHasher {
        fn finish(&self) -> u64 {
            0
        }

        fn write(&mut self, _bytes: &[u8]) {}
    }

    #[test]
    fn complete_hash_collisions_are_total_and_safe() {
        let candidates = [backend("a", 1), backend("b", 1)];
        let mut policy = RingHash::with_hasher(
            config(4, 100),
            BuildHasherDefault::<ConstantHasher>::default(),
        );

        for key in 0..100_u64 {
            let selected = policy.pick(&candidates, &key).unwrap();
            assert!(selected.index() < candidates.len());
        }
        assert_eq!(policy.virtual_node_count(), 8);
    }

    #[test]
    fn distinguishes_empty_from_nonempty_ineligible_slices() {
        let empty: [Backend<&str>; 0] = [];
        let unavailable = [Backend::new("a").with_status(Status::Unavailable)];
        let mut policy = RingHash::new(config(4, 100));

        assert_eq!(policy.pick(&empty, &0_u64), Err(PickError::Empty));
        assert_eq!(
            policy.pick(&unavailable, &0_u64),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
