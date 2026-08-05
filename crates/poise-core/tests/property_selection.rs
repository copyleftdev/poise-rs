//! Public-API properties shared by general selection policies.

mod support;

use std::num::{NonZeroU32, NonZeroUsize};

use poise_core::policy::{
    BoundedLoadRendezvous, LeastLoaded, Maglev, MaglevConfig, PowerOfTwoChoices, Random,
    Rendezvous, RingHash, RingHashConfig, RoundRobin, SmoothWeightedRoundRobin, WeightedRandom,
    WeightedRendezvous,
};
use poise_core::{Backend, Candidate, Policy, Weight};
use proptest::prelude::*;

use support::{assert_valid_selection, backends, candidate_specs, property_config};

fn is_prime_reference(value: usize) -> bool {
    value >= 2 && (2..value).all(|divisor| value % divisor != 0)
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn every_general_policy_returns_only_an_eligible_in_bounds_selection(
        specs in candidate_specs(),
        seed in any::<u64>(),
        key in any::<u64>(),
    ) {
        let candidates = backends(&specs);
        let cursor = usize::try_from(seed).unwrap_or(usize::MAX);

        assert_valid_selection(
            RoundRobin::with_cursor(cursor).pick(&candidates, &()),
            &candidates,
        )?;
        assert_valid_selection(Random::seeded(seed).pick(&candidates, &()), &candidates)?;
        assert_valid_selection(
            WeightedRandom::seeded(seed).pick(&candidates, &()),
            &candidates,
        )?;
        assert_valid_selection(LeastLoaded::new().pick(&candidates, &()), &candidates)?;
        assert_valid_selection(
            PowerOfTwoChoices::seeded(seed).pick(&candidates, &()),
            &candidates,
        )?;
        assert_valid_selection(Rendezvous::new().pick(&candidates, &key), &candidates)?;
        assert_valid_selection(
            WeightedRendezvous::new().pick(&candidates, &key),
            &candidates,
        )?;
        assert_valid_selection(
            SmoothWeightedRoundRobin::new().pick(&candidates, &()),
            &candidates,
        )?;

        let ring_config = RingHashConfig::new(
            NonZeroU32::new(3).unwrap(),
            NonZeroUsize::new(1_024).unwrap(),
        ).unwrap();
        assert_valid_selection(
            RingHash::<u32>::new(ring_config).pick(&candidates, &key),
            &candidates,
        )?;
        assert_valid_selection(
            Maglev::<u32>::new(MaglevConfig::new(101).unwrap()).pick(&candidates, &key),
            &candidates,
        )?;
        assert_valid_selection(
            BoundedLoadRendezvous::default().pick(&candidates, &key),
            &candidates,
        )?;
    }

    #[test]
    fn least_loaded_always_returns_the_minimum_eligible_sample(specs in candidate_specs()) {
        let candidates = backends(&specs);
        let result = LeastLoaded::new().pick(&candidates, &());
        assert_valid_selection(result, &candidates)?;

        if let Ok(selection) = result {
            let minimum = candidates
                .iter()
                .filter(|candidate| candidate.is_eligible())
                .map(|candidate| *candidate.load())
                .min()
                .unwrap();
            prop_assert_eq!(*candidates[selection.index()].load(), minimum);
        }
    }

    #[test]
    fn round_robin_visits_each_eligible_member_once_per_cycle(
        specs in candidate_specs(),
        cursor in any::<usize>(),
    ) {
        let candidates = backends(&specs);
        let eligible: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| candidate.is_eligible().then_some(index))
            .collect();
        let mut policy = RoundRobin::with_cursor(cursor);
        let mut selected = Vec::new();
        for _ in 0..eligible.len() {
            selected.push(policy.pick(&candidates, &()).unwrap().index());
        }
        selected.sort_unstable();
        prop_assert_eq!(selected, eligible);
    }

    #[test]
    fn smooth_weighted_round_robin_conserves_exact_cycle_weight(
        weights in proptest::collection::vec(1_u32..=20, 1..9),
    ) {
        let candidates: Vec<_> = weights
            .iter()
            .enumerate()
            .map(|(index, weight)| {
                Backend::new(u32::try_from(index).unwrap())
                    .with_weight(Weight::new(*weight).unwrap())
            })
            .collect();
        let selections = weights.iter().map(|weight| *weight as usize).sum::<usize>();
        let mut counts = vec![0_u32; candidates.len()];
        let mut policy = SmoothWeightedRoundRobin::new();
        for _ in 0..selections {
            counts[policy.pick(&candidates, &()).unwrap().index()] += 1;
        }
        prop_assert_eq!(counts, weights);
    }

    #[test]
    fn seeded_stochastic_policies_replay_exactly(
        specs in candidate_specs(),
        seed in any::<u64>(),
    ) {
        let candidates = backends(&specs);
        let mut random_left = Random::seeded(seed);
        let mut random_right = Random::seeded(seed);
        let mut weighted_left = WeightedRandom::seeded(seed);
        let mut weighted_right = WeightedRandom::seeded(seed);
        let mut p2c_left = PowerOfTwoChoices::seeded(seed);
        let mut p2c_right = PowerOfTwoChoices::seeded(seed);

        for _ in 0..32 {
            prop_assert_eq!(
                random_left.pick(&candidates, &()),
                random_right.pick(&candidates, &()),
            );
            prop_assert_eq!(
                weighted_left.pick(&candidates, &()),
                weighted_right.pick(&candidates, &()),
            );
            prop_assert_eq!(
                p2c_left.pick(&candidates, &()),
                p2c_right.pick(&candidates, &()),
            );
        }
    }

    #[test]
    fn maglev_configuration_matches_reference_primality(value in 0_usize..10_000) {
        prop_assert_eq!(MaglevConfig::new(value).is_ok(), is_prime_reference(value));
    }
}
