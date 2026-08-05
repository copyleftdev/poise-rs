use std::{
    collections::HashSet,
    error::Error,
    fmt,
    hash::{BuildHasher, Hash, Hasher},
    num::NonZeroUsize,
};

use crate::{Candidate, FnvBuildHasher, PickError, Policy, Selection, mix64};

use super::no_candidate_error;

/// Default number of entries in a [`Maglev`] lookup table.
pub const DEFAULT_TABLE_SIZE: usize = 65_537;

/// Largest lookup table accepted by [`MaglevConfig`].
///
/// The bound keeps rebuild work and memory explicit and matches Envoy's
/// operational limit for its Maglev policy.
pub const MAX_TABLE_SIZE: usize = 5_000_011;

/// Memory and distribution policy for [`Maglev`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MaglevConfig {
    table_size: NonZeroUsize,
}

impl MaglevConfig {
    /// Creates a configuration with a prime-sized lookup table.
    ///
    /// # Errors
    ///
    /// Returns [`MaglevConfigError`] if `table_size` is smaller than two,
    /// larger than [`MAX_TABLE_SIZE`], or not prime.
    pub fn new(table_size: usize) -> Result<Self, MaglevConfigError> {
        if table_size < 2 {
            return Err(MaglevConfigError::TooSmall);
        }
        if table_size > MAX_TABLE_SIZE {
            return Err(MaglevConfigError::TooLarge);
        }
        if !is_prime(table_size) {
            return Err(MaglevConfigError::NotPrime);
        }
        let Some(table_size) = NonZeroUsize::new(table_size) else {
            return Err(MaglevConfigError::TooSmall);
        };
        Ok(Self { table_size })
    }

    /// Returns the number of entries in a populated lookup table.
    #[must_use]
    pub const fn table_size(self) -> usize {
        self.table_size.get()
    }
}

impl Default for MaglevConfig {
    fn default() -> Self {
        Self {
            table_size: NonZeroUsize::new(DEFAULT_TABLE_SIZE)
                .expect("the default table size is non-zero"),
        }
    }
}

/// Invalid Maglev configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MaglevConfigError {
    /// A permutation table needs at least two entries.
    TooSmall,
    /// The requested table exceeds [`MAX_TABLE_SIZE`].
    TooLarge,
    /// The table size is not prime.
    NotPrime,
}

impl fmt::Display for MaglevConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall => formatter.write_str("Maglev table size must be at least two"),
            Self::TooLarge => formatter.write_str("Maglev table size exceeds the supported limit"),
            Self::NotPrime => formatter.write_str("Maglev table size must be prime"),
        }
    }
}

impl Error for MaglevConfigError {}

/// Result of reconciling a candidate slice with a cached Maglev table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MaglevUpdate {
    /// Identity, eligibility, index, and order matched the cached state.
    Unchanged,
    /// A new table was committed.
    Rebuilt {
        /// Number of eligible candidate identities.
        members: usize,
        /// Number of populated table entries, or zero for an empty set.
        table_size: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Member<Key> {
    key: Key,
    index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Permutation {
    offset: usize,
    skip: usize,
    index: usize,
    order: (u64, u64),
}

/// An unweighted, minimally disruptive Maglev lookup table.
///
/// Every eligible identity receives a deterministic permutation of a fixed,
/// prime-sized table. Reconciliation fills that table nearly evenly, and a
/// request lookup is one hash and one array access. Construction is `O(m + n)`
/// memory for `m` table entries and `n` members. Each safe [`Policy::pick`]
/// performs `O(n)` membership validation followed by `O(1)` lookup; unchanged
/// picks allocate nothing and do not rebuild.
///
/// `Maglev` is deliberately unweighted: candidate weights do not affect its
/// cache or selection. Use [`super::WeightedRendezvous`] or
/// [`super::RingHash`] when capacity weights must influence affinity.
///
/// Rebuilds are staged. Duplicate eligible identities, more eligible members
/// than table entries, or allocation failure returns a [`PickError`] without
/// replacing the previous table.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, policy::Maglev};
///
/// let backends = [Backend::new("west"), Backend::new("east")];
/// let mut policy = Maglev::<&str>::default();
/// let first = policy.pick(&backends, &"account-42").unwrap();
/// let replay = policy.pick(&backends, &"account-42").unwrap();
/// assert_eq!(first, replay);
/// assert_eq!(policy.generation(), 1);
/// ```
#[derive(Clone, Debug)]
pub struct Maglev<Key, S = FnvBuildHasher> {
    config: MaglevConfig,
    hash_builder: S,
    members: Vec<Member<Key>>,
    table: Vec<usize>,
    generation: u64,
}

impl<Key> Maglev<Key, FnvBuildHasher> {
    /// Creates an empty deterministic Maglev policy.
    #[must_use]
    pub fn new(config: MaglevConfig) -> Self {
        Self::with_hasher(config, FnvBuildHasher::default())
    }
}

impl<Key> Default for Maglev<Key, FnvBuildHasher> {
    fn default() -> Self {
        Self::new(MaglevConfig::default())
    }
}

impl<Key, S> Maglev<Key, S> {
    /// Creates an empty policy with a caller-provided hash builder.
    #[must_use]
    pub const fn with_hasher(config: MaglevConfig, hash_builder: S) -> Self {
        Self {
            config,
            hash_builder,
            members: Vec::new(),
            table: Vec::new(),
            generation: 0,
        }
    }

    /// Returns the configuration.
    #[must_use]
    pub const fn config(&self) -> MaglevConfig {
        self.config
    }

    /// Returns the hash builder.
    #[must_use]
    pub const fn hash_builder(&self) -> &S {
        &self.hash_builder
    }

    /// Returns the committed eligible-member count.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Returns the committed table length, or zero when no member is eligible.
    #[must_use]
    pub fn populated_table_size(&self) -> usize {
        self.table.len()
    }

    /// Returns the number of committed rebuilds, wrapping at `u64::MAX`.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Clears the cached membership and table.
    pub fn reset(&mut self) {
        self.members.clear();
        self.table.clear();
        self.generation = 0;
    }

    /// Decomposes the policy into configuration and hash builder.
    #[must_use]
    pub fn into_parts(self) -> (MaglevConfig, S) {
        (self.config, self.hash_builder)
    }
}

impl<Key, S> Maglev<Key, S>
where
    Key: Clone + Eq + Hash,
    S: BuildHasher,
{
    /// Rebuilds the table if identity, slice index, order, or eligibility changed.
    ///
    /// Candidate weights are intentionally ignored.
    ///
    /// # Errors
    ///
    /// Returns [`PickError::DuplicateIdentity`] for duplicate eligible keys or
    /// [`PickError::StateCapacityExceeded`] if there are more members than
    /// table entries or the allocator cannot hold the staged state.
    pub fn reconcile<C>(&mut self, candidates: &[C]) -> Result<MaglevUpdate, PickError>
    where
        C: Candidate<Id = Key>,
    {
        if self.matches(candidates) {
            return Ok(MaglevUpdate::Unchanged);
        }

        let eligible_count = candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
            .count();
        if eligible_count > self.config.table_size() {
            return Err(PickError::StateCapacityExceeded);
        }

        let mut seen = HashSet::new();
        seen.try_reserve(eligible_count)
            .map_err(|_| PickError::StateCapacityExceeded)?;
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.is_eligible())
        {
            if !seen.insert(candidate.id()) {
                return Err(PickError::DuplicateIdentity);
            }
        }

        let mut next_members = Vec::new();
        next_members
            .try_reserve_exact(eligible_count)
            .map_err(|_| PickError::StateCapacityExceeded)?;
        let mut permutations = Vec::new();
        permutations
            .try_reserve_exact(eligible_count)
            .map_err(|_| PickError::StateCapacityExceeded)?;

        let table_size = self.config.table_size();
        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }
            next_members.push(Member {
                key: candidate.id().clone(),
                index,
            });
            permutations.push(Permutation {
                offset: reduce_hash(
                    hash_identity(&self.hash_builder, 6, candidate.id()),
                    table_size,
                ),
                skip: reduce_hash(
                    hash_identity(&self.hash_builder, 7, candidate.id()),
                    table_size - 1,
                ) + 1,
                index,
                order: (
                    hash_identity(&self.hash_builder, 8, candidate.id()),
                    hash_identity(&self.hash_builder, 9, candidate.id()),
                ),
            });
        }
        permutations.sort_unstable_by_key(|permutation| (permutation.order, permutation.index));

        let next_table = populate_table(table_size, &permutations)?;
        self.members = next_members;
        self.table = next_table;
        self.generation = self.generation.wrapping_add(1);
        Ok(MaglevUpdate::Rebuilt {
            members: self.members.len(),
            table_size: self.table.len(),
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
            if member.index != index || member.key != *candidate.id() {
                return false;
            }
        }
        members.next().is_none()
    }
}

impl<C, Key, Context, S> Policy<C, Context> for Maglev<Key, S>
where
    C: Candidate<Id = Key>,
    Key: Clone + Eq + Hash,
    Context: Hash + ?Sized,
    S: BuildHasher,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        self.reconcile(candidates)?;
        if self.table.is_empty() {
            return Err(no_candidate_error(candidates.len()));
        }

        let slot = reduce_hash(
            hash_identity(&self.hash_builder, 10, context),
            self.table.len(),
        );
        Ok(Selection::new(self.table[slot]))
    }
}

fn populate_table(
    table_size: usize,
    permutations: &[Permutation],
) -> Result<Vec<usize>, PickError> {
    if permutations.is_empty() {
        return Ok(Vec::new());
    }
    if permutations.len() > table_size {
        return Err(PickError::StateCapacityExceeded);
    }

    let mut next = Vec::new();
    next.try_reserve_exact(permutations.len())
        .map_err(|_| PickError::StateCapacityExceeded)?;
    next.resize(permutations.len(), 0_usize);

    let mut table = Vec::new();
    table
        .try_reserve_exact(table_size)
        .map_err(|_| PickError::StateCapacityExceeded)?;
    table.resize(table_size, usize::MAX);

    let mut populated = 0_usize;
    while populated < table_size {
        for (permutation_index, permutation) in permutations.iter().enumerate() {
            let mut slot = permutation_slot(*permutation, next[permutation_index], table_size);
            while table[slot] != usize::MAX {
                next[permutation_index] += 1;
                slot = permutation_slot(*permutation, next[permutation_index], table_size);
            }
            table[slot] = permutation.index;
            next[permutation_index] = next[permutation_index]
                .checked_add(1)
                .ok_or(PickError::StateCapacityExceeded)?;
            populated += 1;
            if populated == table_size {
                break;
            }
        }
    }
    Ok(table)
}

fn permutation_slot(permutation: Permutation, ordinal: usize, table_size: usize) -> usize {
    let offset = u64::try_from(permutation.offset).expect("configured table size fits u64");
    let skip = u64::try_from(permutation.skip).expect("configured table size fits u64");
    let ordinal = u64::try_from(ordinal).expect("a permutation ordinal fits u64");
    let table_size = u64::try_from(table_size).expect("configured table size fits u64");
    usize::try_from((offset + ordinal * skip) % table_size)
        .expect("a table slot is representable as usize")
}

fn hash_identity<S, Value>(hash_builder: &S, domain: u8, value: &Value) -> u64
where
    S: BuildHasher,
    Value: Hash + ?Sized,
{
    let mut hasher = hash_builder.build_hasher();
    hasher.write_u8(domain);
    value.hash(&mut hasher);
    mix64(hasher.finish())
}

fn reduce_hash(hash: u64, modulus: usize) -> usize {
    let modulus = u64::try_from(modulus).expect("configured table size fits u64");
    usize::try_from(hash % modulus).expect("a reduced hash is representable as usize")
}

fn is_prime(value: usize) -> bool {
    if value < 2 {
        return false;
    }
    if value % 2 == 0 {
        return value == 2;
    }
    let mut divisor = 3_usize;
    while divisor <= value / divisor {
        if value % divisor == 0 {
            return false;
        }
        divisor += 2;
    }
    true
}

#[cfg(test)]
mod tests {
    use std::hash::{BuildHasherDefault, Hasher};

    use crate::{Backend, Status, Weight};

    use super::*;

    fn config(table_size: usize) -> MaglevConfig {
        MaglevConfig::new(table_size).unwrap()
    }

    fn backend(id: &'static str, weight: u32) -> Backend<&'static str> {
        Backend::new(id).with_weight(Weight::new(weight).unwrap())
    }

    #[test]
    fn configuration_requires_a_bounded_prime() {
        assert_eq!(MaglevConfig::new(0), Err(MaglevConfigError::TooSmall));
        assert_eq!(MaglevConfig::new(1), Err(MaglevConfigError::TooSmall));
        assert_eq!(MaglevConfig::new(4), Err(MaglevConfigError::NotPrime));
        assert_eq!(
            MaglevConfig::new(MAX_TABLE_SIZE + 1),
            Err(MaglevConfigError::TooLarge)
        );
        assert_eq!(config(2).table_size(), 2);
        assert_eq!(config(DEFAULT_TABLE_SIZE).table_size(), DEFAULT_TABLE_SIZE);
    }

    #[test]
    fn matches_the_published_seven_slot_example() {
        let permutations = [
            Permutation {
                offset: 3,
                skip: 4,
                index: 0,
                order: (0, 0),
            },
            Permutation {
                offset: 0,
                skip: 2,
                index: 1,
                order: (0, 0),
            },
            Permutation {
                offset: 3,
                skip: 1,
                index: 2,
                order: (0, 0),
            },
        ];

        assert_eq!(
            populate_table(7, &permutations).unwrap(),
            [1, 0, 1, 0, 2, 2, 0]
        );
    }

    #[test]
    fn table_is_cached_until_membership_changes() {
        let candidates = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut policy = Maglev::new(config(101));

        assert_eq!(
            policy.reconcile(&candidates),
            Ok(MaglevUpdate::Rebuilt {
                members: 3,
                table_size: 101,
            })
        );
        assert_eq!(policy.member_count(), 3);
        assert_eq!(policy.populated_table_size(), 101);
        assert_eq!(policy.generation(), 1);
        assert_eq!(policy.reconcile(&candidates), Ok(MaglevUpdate::Unchanged));
        for key in 0..1_000_u64 {
            policy.pick(&candidates, &key).unwrap();
        }
        assert_eq!(policy.generation(), 1);
    }

    #[test]
    fn deterministic_permutations_have_a_stable_small_table() {
        let candidates = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut policy = Maglev::new(config(11));
        policy.reconcile(&candidates).unwrap();
        assert_eq!(policy.table, [1, 2, 1, 2, 1, 0, 0, 2, 0, 1, 0]);
    }

    #[test]
    fn table_slots_are_balanced_to_within_one() {
        let candidates = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut policy = Maglev::new(config(DEFAULT_TABLE_SIZE));
        policy.reconcile(&candidates).unwrap();
        let mut counts = [0_usize; 3];
        for index in &policy.table {
            counts[*index] += 1;
        }
        assert_eq!(counts.iter().sum::<usize>(), DEFAULT_TABLE_SIZE);
        assert!(counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1);
    }

    #[test]
    fn reordering_rebuilds_indices_without_changing_winning_identity() {
        let left = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let right = [backend("c", 1), backend("a", 1), backend("b", 1)];
        let mut left_policy = Maglev::new(config(65_537));
        let mut right_policy = Maglev::new(config(65_537));

        for key in 0..20_000_u64 {
            let left_id = left[left_policy.pick(&left, &key).unwrap().index()].id();
            let right_id = right[right_policy.pick(&right, &key).unwrap().index()].id();
            assert_eq!(left_id, right_id);
        }
    }

    #[test]
    fn removing_a_member_preserves_most_other_assignments() {
        let full = [
            backend("a", 1),
            backend("b", 1),
            backend("c", 1),
            backend("d", 1),
        ];
        let reduced = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut full_policy = Maglev::new(config(65_537));
        let mut reduced_policy = Maglev::new(config(65_537));
        let mut moved = 0_u32;

        for key in 0..100_000_u64 {
            let before = full[full_policy.pick(&full, &key).unwrap().index()].id();
            let after = reduced[reduced_policy.pick(&reduced, &key).unwrap().index()].id();
            moved += u32::from(before != after);
        }

        assert!(
            moved > 20_000,
            "removing a quarter should move its keyspace"
        );
        assert!(moved < 40_000, "observed excessive disruption: {moved}");
    }

    #[test]
    fn weights_do_not_rebuild_or_change_unweighted_selection() {
        let unit = [backend("a", 1), backend("b", 1)];
        let changed = [backend("a", 100), backend("b", 1)];
        let mut policy = Maglev::new(config(101));
        let mut before = Vec::new();

        for key in 0..1_000_u64 {
            before.push(policy.pick(&unit, &key).unwrap());
        }
        assert_eq!(policy.reconcile(&changed), Ok(MaglevUpdate::Unchanged));
        for (key, expected) in (0..1_000_u64).zip(before) {
            assert_eq!(policy.pick(&changed, &key).unwrap(), expected);
        }
        assert_eq!(policy.generation(), 1);
    }

    #[test]
    fn eligibility_changes_rebuild_and_exclude_the_candidate() {
        let ready = [backend("a", 1), backend("b", 1)];
        let unavailable = [
            backend("a", 1).with_status(Status::Unavailable),
            backend("b", 1),
        ];
        let mut policy = Maglev::new(config(101));

        policy.pick(&ready, &0_u64).unwrap();
        for key in 0..100_u64 {
            assert_eq!(policy.pick(&unavailable, &key).unwrap().index(), 1);
        }
        assert_eq!(policy.generation(), 2);
    }

    #[test]
    fn duplicate_and_capacity_failures_preserve_the_live_table() {
        let good = [backend("a", 1), backend("b", 1)];
        let duplicate = [backend("same", 1), backend("same", 1)];
        let too_many = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut policy = Maglev::new(config(2));

        policy.reconcile(&good).unwrap();
        let table = policy.table.clone();
        assert_eq!(
            policy.reconcile(&duplicate),
            Err(PickError::DuplicateIdentity)
        );
        assert_eq!(
            policy.reconcile(&too_many),
            Err(PickError::StateCapacityExceeded)
        );
        assert_eq!(policy.table, table);
        assert_eq!(policy.generation(), 1);
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
    fn complete_hash_collisions_are_total_and_balanced() {
        let candidates = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let mut policy =
            Maglev::with_hasher(config(101), BuildHasherDefault::<ConstantHasher>::default());
        policy.reconcile(&candidates).unwrap();

        let mut counts = [0_usize; 3];
        for index in &policy.table {
            counts[*index] += 1;
        }
        assert!(counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1);
        for key in 0..100_u64 {
            assert!(policy.pick(&candidates, &key).unwrap().index() < candidates.len());
        }
    }

    #[test]
    fn distinguishes_empty_from_nonempty_ineligible_slices() {
        let empty: [Backend<&str>; 0] = [];
        let unavailable = [Backend::new("a").with_status(Status::Unavailable)];
        let mut policy = Maglev::new(config(101));

        assert_eq!(policy.pick(&empty, &0_u64), Err(PickError::Empty));
        assert_eq!(
            policy.pick(&unavailable, &0_u64),
            Err(PickError::NoEligibleCandidates)
        );
    }
}
