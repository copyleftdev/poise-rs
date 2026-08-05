//! Determinism and minimal-disruption properties for affinity policies.

mod support;

use std::num::{NonZeroU32, NonZeroUsize};

use poise_core::policy::{
    BoundedLoadRendezvous, Maglev, MaglevConfig, Rendezvous, RingHash, RingHashConfig,
    WeightedRendezvous,
};
use poise_core::{Backend, Policy, Weight};
use proptest::prelude::*;

use support::{backends, candidate_specs, property_config};

fn selected_id<P, Load>(
    policy: &mut P,
    candidates: &[Backend<u32, (), Load>],
    key: u64,
) -> Option<u32>
where
    P: Policy<Backend<u32, (), Load>, u64>,
{
    policy
        .pick(candidates, &key)
        .ok()
        .map(|selection| *candidates[selection.index()].id())
}

fn ring() -> RingHash<u32> {
    RingHash::new(
        RingHashConfig::new(
            NonZeroU32::new(3).unwrap(),
            NonZeroUsize::new(1_024).unwrap(),
        )
        .unwrap(),
    )
}

fn maglev() -> Maglev<u32> {
    Maglev::new(MaglevConfig::new(101).unwrap())
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn affinity_identity_is_independent_of_candidate_order(
        specs in candidate_specs(),
        key in any::<u64>(),
    ) {
        let original = backends(&specs);
        let reordered: Vec<_> = original.iter().rev().cloned().collect();

        prop_assert_eq!(
            selected_id(&mut Rendezvous::new(), &original, key),
            selected_id(&mut Rendezvous::new(), &reordered, key),
        );
        prop_assert_eq!(
            selected_id(&mut WeightedRendezvous::new(), &original, key),
            selected_id(&mut WeightedRendezvous::new(), &reordered, key),
        );
        prop_assert_eq!(
            selected_id(&mut ring(), &original, key),
            selected_id(&mut ring(), &reordered, key),
        );
        prop_assert_eq!(
            selected_id(&mut maglev(), &original, key),
            selected_id(&mut maglev(), &reordered, key),
        );
        prop_assert_eq!(
            selected_id(&mut BoundedLoadRendezvous::default(), &original, key),
            selected_id(&mut BoundedLoadRendezvous::default(), &reordered, key),
        );
    }

    #[test]
    fn rendezvous_removing_a_non_owner_never_moves_the_key(
        weights in proptest::collection::vec(1_u32..=20, 2..16),
        key in any::<u64>(),
    ) {
        let candidates: Vec<_> = weights
            .iter()
            .enumerate()
            .map(|(index, weight)| {
                Backend::new(u32::try_from(index).unwrap())
                    .with_weight(Weight::new(*weight).unwrap())
            })
            .collect();

        let mut unweighted = Rendezvous::new();
        let owner = selected_id(&mut unweighted, &candidates, key).unwrap();
        let removed = *candidates.iter().find(|candidate| *candidate.id() != owner).unwrap().id();
        let remaining: Vec<_> = candidates
            .iter()
            .filter(|candidate| *candidate.id() != removed)
            .cloned()
            .collect();
        prop_assert_eq!(selected_id(&mut unweighted, &remaining, key), Some(owner));

        let mut weighted = WeightedRendezvous::new();
        let owner = selected_id(&mut weighted, &candidates, key).unwrap();
        let removed = *candidates.iter().find(|candidate| *candidate.id() != owner).unwrap().id();
        let remaining: Vec<_> = candidates
            .iter()
            .filter(|candidate| *candidate.id() != removed)
            .cloned()
            .collect();
        prop_assert_eq!(selected_id(&mut weighted, &remaining, key), Some(owner));
    }

    #[test]
    fn rendezvous_addition_can_only_preserve_the_owner_or_choose_the_new_member(
        weights in proptest::collection::vec(1_u32..=20, 1..16),
        new_weight in 1_u32..=20,
        key in any::<u64>(),
    ) {
        let mut candidates: Vec<_> = weights
            .iter()
            .enumerate()
            .map(|(index, weight)| {
                Backend::new(u32::try_from(index).unwrap())
                    .with_weight(Weight::new(*weight).unwrap())
            })
            .collect();
        let new_id = u32::try_from(candidates.len()).unwrap();

        let old_unweighted = selected_id(&mut Rendezvous::new(), &candidates, key).unwrap();
        let old_weighted = selected_id(&mut WeightedRendezvous::new(), &candidates, key).unwrap();
        candidates.push(Backend::new(new_id).with_weight(Weight::new(new_weight).unwrap()));

        let new_unweighted = selected_id(&mut Rendezvous::new(), &candidates, key).unwrap();
        let new_weighted = selected_id(&mut WeightedRendezvous::new(), &candidates, key).unwrap();
        prop_assert!(new_unweighted == old_unweighted || new_unweighted == new_id);
        prop_assert!(new_weighted == old_weighted || new_weighted == new_id);
    }

    #[test]
    fn equal_weights_make_weighted_and_unweighted_rendezvous_identical(
        specs in candidate_specs(),
        common_weight in 1_u32..=u32::MAX,
        key in any::<u64>(),
    ) {
        let candidates: Vec<_> = backends(&specs)
            .into_iter()
            .map(|candidate| candidate.with_weight(Weight::new(common_weight).unwrap()))
            .collect();
        prop_assert_eq!(
            selected_id(&mut Rendezvous::new(), &candidates, key),
            selected_id(&mut WeightedRendezvous::new(), &candidates, key),
        );
    }
}
