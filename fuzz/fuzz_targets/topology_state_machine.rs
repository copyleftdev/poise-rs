#![no_main]

use libfuzzer_sys::{
    arbitrary::{self, Arbitrary},
    fuzz_target,
};
use poise_core::{
    Backend, Candidate, PickError, Status, Weight,
    policy::{
        LocalityWeightedRandom, Localized, PanicMode, Prioritized, PriorityCandidate,
        PriorityConfig, PriorityMode, PriorityWeightedRandom,
    },
};

#[derive(Arbitrary, Debug)]
struct Node {
    priority: u8,
    locality: u8,
    status: u8,
    endpoint_weight: u32,
    locality_weight: u32,
    panic_eligible: bool,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    nodes: Vec<Node>,
    seed: u64,
    overprovisioning: u8,
    panic_threshold: u8,
    fail_closed: bool,
    repetitions: u8,
}

fn status(value: u8) -> Status {
    match value % 3 {
        0 => Status::Ready,
        1 => Status::Draining,
        _ => Status::Unavailable,
    }
}

fn weight(value: u32) -> Weight {
    Weight::new(value.max(1)).unwrap()
}

fuzz_target!(|scenario: Scenario| {
    if scenario.nodes.len() > 32 {
        return;
    }

    let candidates: Vec<_> = scenario
        .nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            Localized::new(
                Prioritized::new(
                    Backend::new(u8::try_from(index).unwrap())
                        .with_weight(weight(node.endpoint_weight))
                        .with_status(status(node.status)),
                    u32::from(node.priority % 4),
                )
                .with_panic_eligibility(node.panic_eligible),
                node.locality % 4,
            )
            .with_locality_weight(weight(node.locality_weight))
        })
        .collect();

    let mode = if scenario.fail_closed {
        PanicMode::FailClosed
    } else {
        PanicMode::UseAll
    };
    let config = PriorityConfig::new(
        100 + u32::from(scenario.overprovisioning),
        u32::from(scenario.panic_threshold % 101),
        mode,
    )
    .unwrap();
    let mut priority = PriorityWeightedRandom::seeded_with(config, scenario.seed);
    let mut locality = LocalityWeightedRandom::seeded_with(config, scenario.seed);

    for _ in 0..=scenario.repetitions.min(16) {
        match priority.decide(&candidates) {
            Ok(decision) => {
                let candidate = &candidates[decision.selection().index()];
                assert!(candidate.is_priority_member());
                assert_eq!(candidate.priority(), decision.priority());
                match decision.mode() {
                    PriorityMode::Healthy => assert!(candidate.is_eligible()),
                    PriorityMode::Panic => assert!(candidate.is_panic_eligible()),
                }
            }
            Err(
                PickError::Empty
                | PickError::NoEligibleCandidates
                | PickError::PanicRejected
                | PickError::WeightOverflow
                | PickError::StateCapacityExceeded,
            ) => {}
            Err(error) => panic!("unexpected priority error: {error}"),
        }

        match locality.decide(&candidates) {
            Ok(decision) => {
                let candidate = &candidates[decision.selection().index()];
                assert!(candidate.is_priority_member());
                assert_eq!(candidate.priority(), decision.priority());
                assert_eq!(candidate.locality_weight(), decision.locality_weight());
                match decision.priority_mode() {
                    PriorityMode::Healthy => assert!(candidate.is_eligible()),
                    PriorityMode::Panic => assert!(candidate.is_panic_eligible()),
                }
            }
            Err(
                PickError::Empty
                | PickError::NoEligibleCandidates
                | PickError::PanicRejected
                | PickError::WeightOverflow
                | PickError::StateCapacityExceeded
                | PickError::InconsistentTopology,
            ) => {}
            Err(error) => panic!("unexpected locality error: {error}"),
        }
    }
});
