//! Priority, locality, health-mode, and replay properties.

mod support;

use poise_core::policy::{
    LocalityWeightedRandom, Localized, Prioritized, PriorityCandidate, PriorityMode,
    PriorityWeightedRandom,
};
use poise_core::{Backend, Candidate, PickError, Status, Weight};
use proptest::prelude::*;

use support::{CandidateSpec, candidate_specs, property_config};

type PriorityBackend = Prioritized<Backend<u32, (), u64>>;
type LocalityBackend = Localized<PriorityBackend, u8>;

fn priority_backends(specs: &[CandidateSpec], priorities: &[u8]) -> Vec<PriorityBackend> {
    specs
        .iter()
        .zip(priorities)
        .enumerate()
        .map(|(index, (spec, priority))| {
            Prioritized::new(
                Backend::new(u32::try_from(index).unwrap())
                    .with_load(spec.load)
                    .with_weight(Weight::new(spec.weight).unwrap())
                    .with_status(spec.status),
                u32::from(*priority),
            )
        })
        .collect()
}

fn locality_backends(
    specs: &[CandidateSpec],
    priorities: &[u8],
    localities: &[u8],
) -> Vec<LocalityBackend> {
    priority_backends(specs, priorities)
        .into_iter()
        .zip(localities)
        .map(|(candidate, locality)| {
            Localized::new(candidate, *locality)
                .with_locality_weight(Weight::new(u32::from(*locality) + 1).unwrap())
        })
        .collect()
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn priority_decisions_obey_the_sampled_health_mode(
        specs in candidate_specs(),
        priorities in proptest::collection::vec(0_u8..5, 0..16),
        seed in any::<u64>(),
    ) {
        let length = specs.len().min(priorities.len());
        let candidates = priority_backends(&specs[..length], &priorities[..length]);
        let decision = PriorityWeightedRandom::seeded(seed).decide(&candidates);

        match decision {
            Ok(decision) => {
                let selected = &candidates[decision.selection().index()];
                prop_assert_eq!(selected.priority(), decision.priority());
                prop_assert_ne!(selected.status(), Status::Draining);
                if decision.mode() == PriorityMode::Healthy {
                    prop_assert!(selected.is_eligible());
                } else {
                    prop_assert!(selected.is_panic_eligible());
                }
            }
            Err(error) => {
                let expected = if candidates.is_empty() {
                    PickError::Empty
                } else {
                    PickError::NoEligibleCandidates
                };
                prop_assert_eq!(error, expected);
                prop_assert!(!candidates.iter().any(|candidate| candidate.status() != Status::Draining));
            }
        }
    }

    #[test]
    fn fully_healthy_lowest_priority_suppresses_all_failover_tiers(
        weights in proptest::collection::vec(1_u32..=20, 1..16),
        priorities in proptest::collection::vec(0_u8..5, 1..16),
        seed in any::<u64>(),
    ) {
        let length = weights.len().min(priorities.len());
        let candidates: Vec<_> = weights[..length]
            .iter()
            .zip(&priorities[..length])
            .enumerate()
            .map(|(index, (weight, priority))| {
                Prioritized::new(
                    Backend::new(u32::try_from(index).unwrap())
                        .with_weight(Weight::new(*weight).unwrap()),
                    u32::from(*priority),
                )
            })
            .collect();
        let expected = u32::from(*priorities[..length].iter().min().unwrap());
        let decision = PriorityWeightedRandom::seeded(seed).decide(&candidates).unwrap();
        prop_assert_eq!(decision.priority(), expected);
        prop_assert_eq!(decision.mode(), PriorityMode::Healthy);
    }

    #[test]
    fn locality_decisions_preserve_priority_health_and_topology_metadata(
        specs in candidate_specs(),
        priorities in proptest::collection::vec(0_u8..5, 0..16),
        localities in proptest::collection::vec(0_u8..5, 0..16),
        seed in any::<u64>(),
    ) {
        let length = specs.len().min(priorities.len()).min(localities.len());
        let candidates = locality_backends(
            &specs[..length],
            &priorities[..length],
            &localities[..length],
        );
        let decision = LocalityWeightedRandom::seeded(seed).decide(&candidates);

        match decision {
            Ok(decision) => {
                let selected = &candidates[decision.selection().index()];
                prop_assert_eq!(selected.priority(), decision.priority());
                prop_assert_eq!(selected.locality_weight(), decision.locality_weight());
                prop_assert_ne!(selected.status(), Status::Draining);
                prop_assert!(decision.effective_locality_weight() > 0);
                if decision.priority_mode() == PriorityMode::Healthy {
                    prop_assert!(selected.is_eligible());
                } else {
                    prop_assert!(selected.is_panic_eligible());
                }
            }
            Err(error) => {
                let expected = if candidates.is_empty() {
                    PickError::Empty
                } else {
                    PickError::NoEligibleCandidates
                };
                prop_assert_eq!(error, expected);
                prop_assert!(!candidates.iter().any(|candidate| candidate.status() != Status::Draining));
            }
        }
    }

    #[test]
    fn topology_policies_replay_seeded_sequences_exactly(
        specs in candidate_specs(),
        priorities in proptest::collection::vec(0_u8..5, 0..16),
        localities in proptest::collection::vec(0_u8..5, 0..16),
        seed in any::<u64>(),
    ) {
        let priority_length = specs.len().min(priorities.len());
        let priority_candidates =
            priority_backends(&specs[..priority_length], &priorities[..priority_length]);
        let locality_length = priority_length.min(localities.len());
        let locality_candidates = locality_backends(
            &specs[..locality_length],
            &priorities[..locality_length],
            &localities[..locality_length],
        );
        let mut priority_left = PriorityWeightedRandom::seeded(seed);
        let mut priority_right = PriorityWeightedRandom::seeded(seed);
        let mut locality_left = LocalityWeightedRandom::seeded(seed);
        let mut locality_right = LocalityWeightedRandom::seeded(seed);

        for _ in 0..32 {
            prop_assert_eq!(
                priority_left.decide(&priority_candidates),
                priority_right.decide(&priority_candidates),
            );
            prop_assert_eq!(
                locality_left.decide(&locality_candidates),
                locality_right.decide(&locality_candidates),
            );
        }
    }
}
