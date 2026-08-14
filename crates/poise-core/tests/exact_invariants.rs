//! Exact laws, checked exactly.
//!
//! Several policy guarantees are theorems rather than tendencies, and a
//! statistical check of a theorem is strictly weaker than the theorem. These
//! tests assert the closed forms directly, exhaustively over a small scope
//! where the scope is finite, so a violation is a counterexample rather than a
//! threshold being crossed.
//!
//! They complement the property suite rather than replacing it: Proptest
//! samples a large space with shrinking, these enumerate a small space
//! completely.

use std::collections::BTreeSet;

use poise_core::{
    Backend, PickError, Policy, Status, Weight,
    policy::{BoundedLoadConfig, BoundedLoadRendezvous, Rendezvous, WeightedRendezvous},
};

type Loaded = Backend<String, (), u64>;

fn loaded(id: usize, weight: u32, load: u64) -> Loaded {
    Backend::from_parts(
        format!("backend-{id}"),
        (),
        load,
        Weight::new(weight).expect("weight is non-zero"),
        Status::Ready,
    )
}

/// The bound the implementation computes for one prospective request.
///
/// `ceil(factor * (total_load + 1) * weight / (100 * total_weight))`, restated
/// here so the test derives the expectation independently of the code under
/// test rather than reusing its arithmetic.
fn prospective_capacity(factor: u32, total_load: u128, weight: u32, total_weight: u128) -> u128 {
    (u128::from(factor) * (total_load + 1) * u128::from(weight)).div_ceil(100 * total_weight)
}

/// Runs a closed assignment loop and returns the final per-candidate loads.
///
/// Each selection increments the chosen candidate's load, so the policy sees
/// the consequences of its own decisions. This is the regime the bound is
/// actually for: the per-decision check is local, and a policy could satisfy it
/// at every step while still concentrating load over a sequence.
fn simulate(factor: u32, weights: &[u32], requests: u64) -> Vec<u64> {
    let config = BoundedLoadConfig::new(factor).expect("factor is at least 100");
    let mut policy = BoundedLoadRendezvous::new(config);
    let mut loads = vec![0_u64; weights.len()];
    let total_weight: u128 = weights.iter().map(|weight| u128::from(*weight)).sum();

    for request in 0..requests {
        let candidates: Vec<Loaded> = weights
            .iter()
            .zip(&loads)
            .enumerate()
            .map(|(index, (weight, load))| loaded(index, *weight, *load))
            .collect();

        let total_load: u128 = loads.iter().map(|load| u128::from(*load)).sum();
        let decision = policy
            .decide(&candidates, &request)
            .expect("a factor of at least 100 always leaves one candidate under its bound");

        let index = decision.selection().index();
        let expected = prospective_capacity(factor, total_load, weights[index], total_weight);

        assert_eq!(
            decision.selected_capacity(),
            expected,
            "capacity disagreed with the closed form at request {request}"
        );
        assert!(
            u128::from(decision.selected_load()) < decision.selected_capacity(),
            "selected a candidate at or above its bound at request {request}"
        );

        loads[index] += 1;
    }

    loads
}

/// No candidate finishes above its weighted share scaled by the balance factor.
///
/// The per-decision rule is `load < ceil(f * (t + 1) * w / (100 * W))`, so a
/// selection leaves that candidate at most at its capacity for the total `t + 1`
/// it saw. Capacity is non-decreasing in total load and a candidate's load only
/// changes when it is selected, so with `T` the final total every candidate
/// satisfies `load <= ceil(f * T * w / (100 * W))`. That is the closed form of
/// the guarantee, and it is what an operator sizing a fleet relies on.
#[test]
fn accumulated_load_never_exceeds_the_balance_factor_share() {
    for factor in [100_u32, 120, 150, 300] {
        for weights in [
            vec![1_u32],
            vec![1, 1],
            vec![1, 3],
            vec![1, 1, 1],
            vec![1, 2, 3],
            vec![5, 1, 1, 1],
            vec![1, 2, 3, 4, 5],
        ] {
            let requests = 400;
            let loads = simulate(factor, &weights, requests);

            let total: u128 = loads.iter().map(|load| u128::from(*load)).sum();
            assert_eq!(total, u128::from(requests), "every request was assigned");

            let total_weight: u128 = weights.iter().map(|weight| u128::from(*weight)).sum();
            for (index, load) in loads.iter().enumerate() {
                let bound = (u128::from(factor) * total * u128::from(weights[index]))
                    .div_ceil(100 * total_weight);
                assert!(
                    u128::from(*load) <= bound,
                    "factor {factor} weights {weights:?}: candidate {index} \
                     carried {load} against a bound of {bound}"
                );
            }
        }
    }
}

/// At the tightest factor, allocation is exactly proportional to weight.
///
/// A balance factor of 100 gives every candidate its weighted share and not one
/// request more, so over `T` assignments each candidate must land within one of
/// `T * w / W` — it cannot round up twice, because the second time it would be
/// at its bound and lose the selection.
///
/// This is the sharp form of the previous law rather than a repetition of it.
/// The `<=` bound is satisfied by any policy that under-uses a candidate, and
/// is therefore only half a specification; this pins the allocation from both
/// sides. It is also what makes the bound test non-vacuous: at factor 100 the
/// observed load reaches the ceiling exactly, so a single misassignment
/// anywhere in the sequence breaks it.
#[test]
fn the_tightest_factor_allocates_exactly_in_proportion_to_weight() {
    for weights in [
        vec![1_u32, 1],
        vec![1, 3],
        vec![1, 2, 3],
        vec![5, 1, 1, 1],
        vec![1, 2, 3, 4, 5],
    ] {
        let requests = 400_u64;
        let loads = simulate(100, &weights, requests);
        let total = u128::from(requests);
        let total_weight: u128 = weights.iter().map(|weight| u128::from(*weight)).sum();

        for (index, load) in loads.iter().enumerate() {
            let weight = u128::from(weights[index]);
            let floor = total * weight / total_weight;
            let ceiling = (total * weight).div_ceil(total_weight);
            let load = u128::from(*load);
            assert!(
                load == floor || load == ceiling,
                "weights {weights:?}: candidate {index} carried {load}, \
                 outside the exact share [{floor}, {ceiling}]"
            );
        }
    }
}

/// A factor of exactly 100 still always admits somebody.
///
/// The tightest legal bound is where totality is least obvious: capacity is the
/// weighted share with no slack, so the claim that some candidate is always
/// under it is doing real work. Enumerated exhaustively over every load vector
/// in a small cube rather than sampled, since the failure would be a specific
/// arrangement rather than a rare draw.
#[test]
fn the_tightest_bound_is_still_total() {
    let weights = [1_u32, 2, 3];
    let config = BoundedLoadConfig::new(100).expect("100 is the tightest legal factor");
    let mut policy = BoundedLoadRendezvous::new(config);

    for first in 0..10_u64 {
        for second in 0..10_u64 {
            for third in 0..10_u64 {
                let candidates = [
                    loaded(0, weights[0], first),
                    loaded(1, weights[1], second),
                    loaded(2, weights[2], third),
                ];
                for key in 0..8_u64 {
                    let decision = policy.decide(&candidates, &key).expect(
                        "a factor of 100 must leave one candidate under its prospective bound",
                    );
                    assert!(u128::from(decision.selected_load()) < decision.selected_capacity());
                }
            }
        }
    }
}

/// Spilling only happens when the affinity owner is genuinely full.
///
/// The rule is "highest-ranked candidate under its bound wins", so a selection
/// that differs from the unconstrained owner implies the owner was at or above
/// its own capacity. Without this, a policy could satisfy the load bound by
/// spilling whenever it liked and still pass every capacity assertion.
#[test]
fn a_spill_implies_the_affinity_owner_was_at_its_bound() {
    let weights = [1_u32, 2, 3];
    let config = BoundedLoadConfig::new(110).expect("110 is a legal factor");
    let mut policy = BoundedLoadRendezvous::new(config);
    let total_weight: u128 = weights.iter().map(|weight| u128::from(*weight)).sum();
    let mut observed_spill = false;

    for first in 0..8_u64 {
        for second in 0..8_u64 {
            for third in 0..8_u64 {
                let loads = [first, second, third];
                let candidates = [
                    loaded(0, weights[0], first),
                    loaded(1, weights[1], second),
                    loaded(2, weights[2], third),
                ];
                let total_load: u128 = loads.iter().map(|load| u128::from(*load)).sum();

                for key in 0..8_u64 {
                    let decision = policy
                        .decide(&candidates, &key)
                        .expect("selection succeeds");
                    if !decision.spilled() {
                        continue;
                    }
                    observed_spill = true;

                    let owner = decision.affinity().index();
                    let owner_capacity =
                        prospective_capacity(110, total_load, weights[owner], total_weight);
                    assert!(
                        u128::from(loads[owner]) >= owner_capacity,
                        "spilled away from candidate {owner} while it was under its bound"
                    );
                }
            }
        }
    }

    assert!(
        observed_spill,
        "the scope produced no spill, so the law was never exercised"
    );
}

/// Identities of the eligible members of a subset, in selection order.
fn members(mask: u32, universe: &[Loaded]) -> Vec<Loaded> {
    (0..universe.len())
        .filter(|index| mask & (1 << index) != 0)
        .map(|index| universe[index].clone())
        .collect()
}

/// Adding a backend moves keys only onto the newcomer, over every subset.
///
/// Rendezvous selects `argmax` of a per-(key, candidate) score, so introducing
/// a candidate can only displace the winner if the newcomer outscores it; two
/// incumbents can never trade a key. The existing suite checks this for one
/// membership. This enumerates every subset of a six-candidate universe and
/// every candidate that could join it, which makes the check complete at that
/// scope rather than a sample of it.
#[test]
fn rendezvous_addition_moves_keys_only_to_the_new_backend() {
    const UNIVERSE: usize = 6;
    let universe: Vec<Loaded> = (0..UNIVERSE).map(|index| loaded(index, 1, 0)).collect();
    let mut policy = Rendezvous::new();

    for mask in 1_u32..(1 << UNIVERSE) {
        let before = members(mask, &universe);
        for addition in 0..UNIVERSE {
            if mask & (1 << addition) != 0 {
                continue;
            }
            let after = members(mask | (1 << addition), &universe);
            let newcomer = universe[addition].id();

            for key in 0..400_u64 {
                let old_id = before[policy.pick(&before, &key).unwrap().index()].id();
                let new_id = after[policy.pick(&after, &key).unwrap().index()].id();
                if new_id != newcomer {
                    assert_eq!(
                        old_id, new_id,
                        "key {key} moved between incumbents when {newcomer} joined"
                    );
                }
            }
        }
    }
}

/// Removing a backend remaps only the keys it owned, over every subset.
#[test]
fn rendezvous_removal_remaps_only_the_departed_keys() {
    const UNIVERSE: usize = 6;
    let universe: Vec<Loaded> = (0..UNIVERSE).map(|index| loaded(index, 1, 0)).collect();
    let mut policy = Rendezvous::new();

    for mask in 1_u32..(1 << UNIVERSE) {
        let before = members(mask, &universe);
        if before.len() < 2 {
            continue;
        }
        for removal in 0..UNIVERSE {
            if mask & (1 << removal) == 0 {
                continue;
            }
            let after = members(mask & !(1 << removal), &universe);
            let departed = universe[removal].id();

            for key in 0..400_u64 {
                let old_id = before[policy.pick(&before, &key).unwrap().index()].id();
                let new_id = after[policy.pick(&after, &key).unwrap().index()].id();
                if old_id != departed {
                    assert_eq!(
                        old_id, new_id,
                        "key {key} moved off {old_id} when unrelated {departed} left"
                    );
                }
            }
        }
    }
}

/// Weighted rendezvous keeps the same displacement law under every subset.
#[test]
fn weighted_rendezvous_addition_moves_keys_only_to_the_new_backend() {
    const UNIVERSE: usize = 5;
    let weights = [1_u32, 2, 3, 5, 8];
    let universe: Vec<Loaded> = (0..UNIVERSE)
        .map(|index| loaded(index, weights[index], 0))
        .collect();
    let mut policy = WeightedRendezvous::new();

    for mask in 1_u32..(1 << UNIVERSE) {
        let before = members(mask, &universe);
        for addition in 0..UNIVERSE {
            if mask & (1 << addition) != 0 {
                continue;
            }
            let after = members(mask | (1 << addition), &universe);
            let newcomer = universe[addition].id();

            for key in 0..400_u64 {
                let old_id = before[policy.pick(&before, &key).unwrap().index()].id();
                let new_id = after[policy.pick(&after, &key).unwrap().index()].id();
                if new_id != newcomer {
                    assert_eq!(
                        old_id, new_id,
                        "key {key} moved between incumbents when {newcomer} joined"
                    );
                }
            }
        }
    }
}

/// Selection is invariant under candidate reordering, over every permutation.
///
/// Rendezvous is defined on identities rather than positions, so the winning
/// identity cannot depend on slice order. Enumerated over all permutations of a
/// five-candidate set rather than a shuffled sample.
#[test]
fn rendezvous_selects_the_same_identity_under_every_permutation() {
    const UNIVERSE: usize = 5;
    let universe: Vec<Loaded> = (0..UNIVERSE).map(|index| loaded(index, 1, 0)).collect();
    let mut policy = Rendezvous::new();

    let mut permutation: Vec<usize> = (0..UNIVERSE).collect();
    let mut permutations = Vec::new();
    permute(&mut permutation, 0, &mut permutations);

    for key in 0..200_u64 {
        let canonical = universe[policy.pick(&universe, &key).unwrap().index()]
            .id()
            .clone();
        for order in &permutations {
            let reordered: Vec<Loaded> =
                order.iter().map(|index| universe[*index].clone()).collect();
            let winner = reordered[policy.pick(&reordered, &key).unwrap().index()].id();
            assert_eq!(&canonical, winner, "key {key} changed winner under reorder");
        }
    }
}

fn permute(items: &mut Vec<usize>, start: usize, out: &mut Vec<Vec<usize>>) {
    if start == items.len() {
        out.push(items.clone());
        return;
    }
    for index in start..items.len() {
        items.swap(start, index);
        permute(items, start + 1, out);
        items.swap(start, index);
    }
}

/// An empty or wholly ineligible slice stays distinguishable at every size.
#[test]
fn empty_and_ineligible_outcomes_stay_distinct_at_every_small_size() {
    let mut policy = Rendezvous::new();
    let empty: [Loaded; 0] = [];
    assert_eq!(policy.pick(&empty, &1_u64), Err(PickError::Empty));

    for size in 1..8_usize {
        let candidates: Vec<Loaded> = (0..size)
            .map(|index| {
                Backend::from_parts(
                    format!("backend-{index}"),
                    (),
                    0_u64,
                    Weight::ONE,
                    Status::Draining,
                )
            })
            .collect();
        assert_eq!(
            policy.pick(&candidates, &1_u64),
            Err(PickError::NoEligibleCandidates),
            "size {size} confused an ineligible slice with an empty one"
        );
    }
}

/// Every candidate owns at least one key once the keyspace is large enough.
///
/// Not a distribution claim, so not a statistical test: this only asserts that
/// no eligible candidate is structurally unreachable, which would indicate a
/// scoring bug rather than an unlucky draw.
#[test]
fn every_eligible_candidate_is_reachable() {
    for size in 2..8_usize {
        let candidates: Vec<Loaded> = (0..size).map(|index| loaded(index, 1, 0)).collect();
        let mut policy = Rendezvous::new();
        let mut winners = BTreeSet::new();

        for key in 0..20_000_u64 {
            winners.insert(
                candidates[policy.pick(&candidates, &key).unwrap().index()]
                    .id()
                    .clone(),
            );
        }

        assert_eq!(
            winners.len(),
            size,
            "some candidate never won a key at size {size}"
        );
    }
}
