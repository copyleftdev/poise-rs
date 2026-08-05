use std::{
    cmp::Ordering,
    hash::{BuildHasher, Hash},
};

use crate::{Candidate, FnvBuildHasher, PickError, Policy, Selection};

use super::{no_candidate_error, rendezvous::rendezvous_hash};

/// Capacity-proportional key affinity using weighted rendezvous hashing.
///
/// Each eligible candidate receives an exponential-race score derived from the
/// request key, stable candidate identity, and configured [`Weight`](crate::Weight):
/// `-ln(U) / weight`. The smallest score wins. This is equivalent to the
/// logarithmic weighted-HRW formulation `-weight / ln(U)` with the largest
/// score winning.
///
/// Selection takes `O(n)` time and `O(1)` memory and allocates nothing. Adding,
/// removing, or reweighting one backend changes only that backend's score, so a
/// key never moves directly between two otherwise unchanged backends. With
/// equal weights, selection exactly matches [`Rendezvous`](super::Rendezvous),
/// including raw-hash tie breaking.
///
/// The default [`FnvBuildHasher`] is deterministic but not collision resistant.
/// Use [`WeightedRendezvous::with_hasher`] when keys or identities are
/// adversarial. The logarithmic transform uses a fixed range reduction and
/// polynomial over the high 53 hash bits rather than platform `libm`; the full
/// raw hash resolves transformed-score ties deterministically.
///
/// Eligible candidate identities should be unique. To retain `O(1)` memory,
/// this policy does not allocate a set to validate them: duplicates receive the
/// same random draw, so the larger weight wins, and an exact weight tie selects
/// the earlier slice entry.
///
/// # Example
///
/// ```
/// use poise_core::{Backend, Policy, Weight, policy::WeightedRendezvous};
///
/// let backends = [
///     Backend::new("small").with_weight(Weight::new(1).unwrap()),
///     Backend::new("large").with_weight(Weight::new(4).unwrap()),
/// ];
/// let mut policy = WeightedRendezvous::new();
/// let first = policy.pick(&backends, &"customer-42").unwrap();
/// let replay = policy.pick(&backends, &"customer-42").unwrap();
/// assert_eq!(first, replay);
/// ```
#[derive(Clone, Debug)]
pub struct WeightedRendezvous<S = FnvBuildHasher> {
    hash_builder: S,
}

impl WeightedRendezvous<FnvBuildHasher> {
    /// Creates a deterministic weighted-rendezvous policy.
    #[must_use]
    pub fn new() -> Self {
        Self::with_hasher(FnvBuildHasher::default())
    }
}

impl<S> WeightedRendezvous<S> {
    /// Creates a policy with a caller-provided hash builder.
    pub const fn with_hasher(hash_builder: S) -> Self {
        Self { hash_builder }
    }

    /// Returns the configured hash builder.
    #[must_use]
    pub const fn hash_builder(&self) -> &S {
        &self.hash_builder
    }

    /// Returns the hash builder, consuming the policy.
    pub fn into_hash_builder(self) -> S {
        self.hash_builder
    }
}

impl Default for WeightedRendezvous<FnvBuildHasher> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Context, S> Policy<C, Context> for WeightedRendezvous<S>
where
    C: Candidate,
    C::Id: Hash,
    Context: Hash + ?Sized,
    S: BuildHasher,
{
    fn pick(&mut self, candidates: &[C], context: &Context) -> Result<Selection, PickError> {
        let mut winner: Option<(usize, f64, u64)> = None;

        for (index, candidate) in candidates.iter().enumerate() {
            if !candidate.is_eligible() {
                continue;
            }

            let hash = rendezvous_hash(&self.hash_builder, context, candidate.id());
            let score = exponential_score(hash, candidate.weight().get());
            let wins = winner.is_none_or(|(_, winning_score, winning_hash)| {
                weighted_score_wins(score, hash, winning_score, winning_hash)
            });
            if wins {
                winner = Some((index, score, hash));
            }
        }

        winner
            .map(|(index, _, _)| Selection::new(index))
            .ok_or_else(|| no_candidate_error(candidates.len()))
    }
}

pub(super) fn exponential_score(hash: u64, weight: u32) -> f64 {
    -log_unit_interval(hash) / f64::from(weight)
}

pub(super) fn weighted_score_wins(
    score: f64,
    hash: u64,
    winning_score: f64,
    winning_hash: u64,
) -> bool {
    match score.total_cmp(&winning_score) {
        Ordering::Less => true,
        Ordering::Equal => hash > winning_hash,
        Ordering::Greater => false,
    }
}

fn log_unit_interval(hash: u64) -> f64 {
    const EXACT_BITS: u32 = 53;
    const DISCARD_BITS: u32 = u64::BITS - EXACT_BITS;
    const LN_2: f64 = std::f64::consts::LN_2;
    const TERMS: u32 = 13;

    // Adding one produces an exactly representable integer in 1..=2^53, so the
    // represented unit value is always in (0, 1].
    let sample = (hash >> DISCARD_BITS) + 1;
    let exponent = u64::BITS - 1 - sample.leading_zeros();
    let power_of_two = 1_u64 << exponent;
    let mantissa = u53_to_f64(sample) / u53_to_f64(power_of_two);

    // ln(m) = 2 * (z + z^3/3 + z^5/5 + ...), where
    // z = (m - 1)/(m + 1). Since m is in [1, 2), z is in [0, 1/3), and thirteen
    // fixed terms keep truncation below roughly 2e-14 without platform libm.
    let z = (mantissa - 1.0) / (mantissa + 1.0);
    let z_squared = z * z;
    let mut term = z;
    let mut sum = 0.0;
    for index in 0..TERMS {
        sum += term / f64::from(2 * index + 1);
        term *= z_squared;
    }
    let log_mantissa = 2.0 * sum;
    log_mantissa - f64::from(EXACT_BITS - exponent) * LN_2
}

fn u53_to_f64(value: u64) -> f64 {
    const LOW_MASK: u64 = 0xffff_ffff;
    const TWO_POW_32: f64 = 4_294_967_296.0;

    debug_assert!(value <= (1_u64 << 53));
    let high = u32::try_from(value >> 32).expect("the high 21 bits fit u32");
    let low = u32::try_from(value & LOW_MASK).expect("the masked low bits fit u32");
    f64::from(high) * TWO_POW_32 + f64::from(low)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{Backend, Status, Weight, policy::Rendezvous};

    use super::*;

    fn backend(id: &'static str, weight: u32) -> Backend<&'static str> {
        Backend::new(id).with_weight(Weight::new(weight).unwrap())
    }

    #[test]
    fn equal_weights_exactly_match_unweighted_rendezvous() {
        let candidates = [backend("a", 7), backend("b", 7), backend("c", 7)];
        let mut weighted = WeightedRendezvous::new();
        let mut unweighted = Rendezvous::new();

        for key in 0..100_000_u64 {
            assert_eq!(
                weighted.pick(&candidates, &key),
                unweighted.pick(&candidates, &key)
            );
        }
    }

    #[test]
    fn transformed_score_ties_use_the_larger_raw_hash() {
        assert!(weighted_score_wins(1.0, 9, 1.0, 7));
        assert!(!weighted_score_wins(1.0, 7, 1.0, 9));
    }

    #[test]
    fn distribution_tracks_capacity_weights() {
        const KEYS: u64 = 300_000;
        let candidates = [
            backend("small", 1),
            backend("medium", 3),
            backend("large", 6),
        ];
        let mut counts = [0_u64; 3];
        let mut policy = WeightedRendezvous::new();

        for key in 0..KEYS {
            counts[policy.pick(&candidates, &key).unwrap().index()] += 1;
        }

        let expected = [0.1_f64, 0.3, 0.6];
        for (count, expected_share) in counts.into_iter().zip(expected) {
            let actual =
                f64::from(u32::try_from(count).unwrap()) / f64::from(u32::try_from(KEYS).unwrap());
            assert!(
                (actual - expected_share).abs() < 0.01,
                "expected {expected_share:.3}, observed {actual:.3}"
            );
        }
    }

    #[test]
    fn removing_a_backend_only_remaps_its_keys() {
        let all = [backend("a", 1), backend("b", 4), backend("c", 2)];
        let without_b = [backend("a", 1), backend("c", 2)];
        let mut policy = WeightedRendezvous::new();

        for key in 0..50_000_u64 {
            let old_id = all[policy.pick(&all, &key).unwrap().index()].id();
            let new_id = without_b[policy.pick(&without_b, &key).unwrap().index()].id();
            if old_id != &"b" {
                assert_eq!(old_id, new_id, "key {key} moved unnecessarily");
            }
        }
    }

    #[test]
    fn adding_a_backend_only_moves_keys_to_the_new_backend() {
        let before = [backend("a", 1), backend("b", 4)];
        let after = [backend("a", 1), backend("b", 4), backend("c", 2)];
        let mut policy = WeightedRendezvous::new();

        for key in 0..50_000_u64 {
            let old_id = before[policy.pick(&before, &key).unwrap().index()].id();
            let new_id = after[policy.pick(&after, &key).unwrap().index()].id();
            if new_id != &"c" {
                assert_eq!(old_id, new_id, "key {key} moved between existing peers");
            }
        }
    }

    #[test]
    fn changing_one_weight_never_moves_between_unchanged_backends() {
        let before = [backend("a", 1), backend("b", 1), backend("c", 1)];
        let after = [backend("a", 1), backend("b", 9), backend("c", 1)];
        let mut policy = WeightedRendezvous::new();

        for key in 0..50_000_u64 {
            let old_id = before[policy.pick(&before, &key).unwrap().index()].id();
            let new_id = after[policy.pick(&after, &key).unwrap().index()].id();
            if old_id != &"b" && new_id != &"b" {
                assert_eq!(old_id, new_id, "key {key} moved between stable peers");
            }
        }
    }

    #[test]
    fn order_does_not_change_identity_assignments() {
        let left = [backend("a", 1), backend("b", 3), backend("c", u32::MAX)];
        let right = [backend("c", u32::MAX), backend("a", 1), backend("b", 3)];
        let mut policy = WeightedRendezvous::new();

        for key in 0..10_000_u64 {
            let left_id = left[policy.pick(&left, &key).unwrap().index()].id();
            let right_id = right[policy.pick(&right, &key).unwrap().index()].id();
            assert_eq!(left_id, right_id);
        }
    }

    #[test]
    fn decisions_replay_and_exclude_ineligible_candidates() {
        let candidates = [
            backend("a", u32::MAX).with_status(Status::Unavailable),
            backend("b", 1),
            backend("c", 4),
        ];
        let mut left = WeightedRendezvous::new();
        let mut right = WeightedRendezvous::new();
        let mut winners = HashMap::new();

        for key in 0..10_000_u64 {
            let left_selection = left.pick(&candidates, &key).unwrap();
            assert_eq!(left_selection, right.pick(&candidates, &key).unwrap());
            assert_ne!(left_selection.index(), 0);
            *winners.entry(left_selection.index()).or_insert(0_u64) += 1;
        }
        assert_eq!(winners.len(), 2);
    }

    #[test]
    fn distinguishes_empty_from_ineligible() {
        let empty: [Backend<&str>; 0] = [];
        let unavailable = [Backend::new("a").with_status(Status::Unavailable)];
        let mut policy = WeightedRendezvous::new();

        assert_eq!(policy.pick(&empty, &0_u64), Err(PickError::Empty));
        assert_eq!(
            policy.pick(&unavailable, &0_u64),
            Err(PickError::NoEligibleCandidates)
        );
    }

    #[test]
    fn duplicate_identity_behavior_is_explicit_and_allocation_free() {
        let weighted_duplicates = [backend("same", 1), backend("same", 2)];
        let equal_duplicates = [backend("same", 2), backend("same", 2)];
        let mut policy = WeightedRendezvous::new();

        for key in 0..100_u64 {
            assert_eq!(policy.pick(&weighted_duplicates, &key).unwrap().index(), 1);
            assert_eq!(policy.pick(&equal_duplicates, &key).unwrap().index(), 0);
        }
    }

    #[test]
    fn fixed_log_transform_tracks_the_standard_library() {
        let hashes = [
            0,
            1,
            0x0123_4567_89ab_cdef,
            0x8000_0000_0000_0000,
            u64::MAX - 1,
            u64::MAX,
        ];

        for hash in hashes {
            let sample = (hash >> 11) + 1;
            let unit = u53_to_f64(sample) / u53_to_f64(1_u64 << 53);
            let approximation = log_unit_interval(hash);
            assert!(
                (approximation - unit.ln()).abs() < 5e-14,
                "hash {hash:#018x}: expected {}, observed {approximation}",
                unit.ln()
            );
        }
    }
}
