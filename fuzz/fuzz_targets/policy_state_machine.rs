#![no_main]

use libfuzzer_sys::{
    arbitrary::{self, Arbitrary},
    fuzz_target,
};
use poise_core::{
    Backend, Candidate, PickError, Policy, Status, Weight,
    policy::{
        BoundedLoadConfig, BoundedLoadRendezvous, LeastLoaded, Maglev, MaglevConfig,
        PowerOfTwoChoices, Random, Rendezvous, RingHash, RingHashConfig, RoundRobin,
        SmoothWeightedRoundRobin, WeightedRandom, WeightedRendezvous,
    },
};
use std::num::{NonZeroU32, NonZeroUsize};

#[derive(Arbitrary, Debug)]
struct Node {
    status: u8,
    weight: u32,
    load: u64,
}

#[derive(Arbitrary, Debug)]
enum Action {
    Pick {
        key: u64,
    },
    Update {
        index: u8,
        status: u8,
        weight: u32,
        load: u64,
    },
    Rotate {
        amount: u8,
    },
    Reverse,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    nodes: Vec<Node>,
    actions: Vec<Action>,
    seed: u64,
}

type TestBackend = Backend<u8, (), u64>;

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

fn assert_result(result: Result<poise_core::Selection, PickError>, candidates: &[TestBackend]) {
    match result {
        Ok(selection) => {
            assert!(selection.index() < candidates.len());
            assert!(candidates[selection.index()].is_eligible());
        }
        Err(PickError::Empty) => assert!(candidates.is_empty()),
        Err(PickError::NoEligibleCandidates) => {
            assert!(!candidates.is_empty());
            assert!(!candidates.iter().any(Candidate::is_eligible));
        }
        Err(PickError::LoadOverflow | PickError::StateCapacityExceeded) => {}
        Err(error) => panic!("unexpected policy error: {error}"),
    }
}

fuzz_target!(|scenario: Scenario| {
    if scenario.nodes.len() > 32 || scenario.actions.len() > 128 {
        return;
    }

    let mut candidates: Vec<TestBackend> = scenario
        .nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            Backend::new(u8::try_from(index).unwrap())
                .with_load(node.load)
                .with_weight(weight(node.weight))
                .with_status(status(node.status))
        })
        .collect();

    let mut round_robin = RoundRobin::with_cursor(usize::from(scenario.seed as u16));
    let mut random = Random::seeded(scenario.seed);
    let mut weighted_random = WeightedRandom::seeded(scenario.seed);
    let mut least_loaded = LeastLoaded::new();
    let mut power_of_two = PowerOfTwoChoices::seeded(scenario.seed);
    let mut rendezvous = Rendezvous::new();
    let mut weighted_rendezvous = WeightedRendezvous::new();
    let mut bounded = BoundedLoadRendezvous::new(BoundedLoadConfig::new(100).unwrap());
    let mut ring = RingHash::<u8>::new(
        RingHashConfig::new(
            NonZeroU32::new(4).unwrap(),
            NonZeroUsize::new(4096).unwrap(),
        )
        .unwrap(),
    );
    let mut maglev = Maglev::<u8>::new(MaglevConfig::new(67).unwrap());
    let mut smooth = SmoothWeightedRoundRobin::<u8>::new();

    let mut exercise = |key: u64, candidates: &[TestBackend]| {
        assert_result(round_robin.pick(candidates, &()), candidates);
        assert_result(random.pick(candidates, &()), candidates);
        assert_result(weighted_random.pick(candidates, &()), candidates);
        assert_result(least_loaded.pick(candidates, &()), candidates);
        assert_result(power_of_two.pick(candidates, &()), candidates);
        assert_result(rendezvous.pick(candidates, &key), candidates);
        assert_result(weighted_rendezvous.pick(candidates, &key), candidates);
        assert_result(bounded.pick(candidates, &key), candidates);
        assert_result(ring.pick(candidates, &key), candidates);
        assert_result(maglev.pick(candidates, &key), candidates);
        assert_result(smooth.pick(candidates, &()), candidates);
    };

    exercise(scenario.seed, &candidates);
    for action in scenario.actions {
        match action {
            Action::Pick { key } => exercise(key, &candidates),
            Action::Update {
                index,
                status: next_status,
                weight: next_weight,
                load,
            } => {
                if !candidates.is_empty() {
                    let index = usize::from(index) % candidates.len();
                    candidates[index].set_status(status(next_status));
                    candidates[index].set_weight(weight(next_weight));
                    candidates[index].set_load(load);
                }
            }
            Action::Rotate { amount } => {
                if !candidates.is_empty() {
                    let amount = usize::from(amount) % candidates.len();
                    candidates.rotate_left(amount);
                }
            }
            Action::Reverse => candidates.reverse(),
        }
    }
    exercise(scenario.seed.rotate_left(17), &candidates);
});
