//! Cached policies checked against the naive implementation of themselves.
//!
//! The verification bar asks for a comparison against a reference
//! implementation. For most policies here that would be a copy: a reference for
//! rendezvous is `argmax` over per-candidate hashes, which is what the shipped
//! code already is, and a test comparing an expression to itself proves only
//! that it was typed twice.
//!
//! The comparison has teeth exactly where the shipped implementation is
//! optimised away from its specification. `RingHash`, `Maglev`, and
//! `BoundedLoadRendezvous` retain a table or a scratch buffer across calls and
//! rebuild it transactionally when membership changes. Their specification is
//! that a decision depends on the candidates and the context and on nothing
//! else. The naive implementation of that specification is a policy with no
//! history at all, so the reference here is a *freshly constructed* policy, and
//! the law is that an aged one must agree with it everywhere.
//!
//! That targets the failure caching introduces and nothing else: a table that
//! survived a membership change it should not have, a fingerprint that missed a
//! weight, scratch reused across differently shaped inputs. None of those are
//! visible from a single call, which is why the property tests do not catch
//! them and why this file drives churn before it compares.
//!
//! Policies excluded on purpose:
//!
//! - Round robin and smooth weighted round robin carry a cursor and per-identity
//!   credit. Their state *is* their behaviour, so a fresh instance is a
//!   different policy rather than a reference for this one.
//! - The randomised policies advance an RNG per call, so a fresh instance
//!   differs by construction. Their laws are distributional and live in
//!   `distribution_power.rs`.
//! - Least loaded keeps only a tie-breaking cursor, which is deliberately
//!   path-dependent for exactly the inputs where the choice is arbitrary.

use poise_core::{
    Backend, PickError, Policy, Selection, Status, Weight,
    policy::{BoundedLoadRendezvous, Maglev, RingHash},
};

type Candidate = Backend<String, (), u64>;

fn candidate(index: usize, weight: u32, load: u64) -> Candidate {
    Backend::from_parts(
        format!("backend-{index:03}"),
        (),
        load,
        Weight::new(weight).expect("weight is non-zero"),
        Status::Ready,
    )
}

/// A membership generation: which members exist, and how they are configured.
///
/// Deliberately varies weight, load, status, and order as well as membership,
/// because a fingerprint that watches identity alone would pass a test that
/// only added and removed members.
fn generation(step: usize, size: usize) -> Vec<Candidate> {
    let mut members: Vec<Candidate> = (0..size)
        .filter(|index| (index + step) % 7 != 0)
        .map(|index| {
            let weight = u32::try_from(1 + (index + step) % 4).unwrap_or(1);
            let load = u64::try_from((index * 3 + step) % 11).unwrap_or(0);
            let mut member = candidate(index, weight, load);
            if (index + step) % 5 == 0 {
                member = member.with_status(Status::Draining);
            }
            member
        })
        .collect();

    if step % 3 == 0 {
        members.reverse();
    }
    members
}

fn keys() -> Vec<String> {
    (0..250).map(|index| format!("key-{index:04}")).collect()
}

/// Drives `policy` through membership churn so its cache carries history.
fn age<P>(policy: &mut P, size: usize)
where
    P: Policy<Candidate, str>,
{
    let request_keys = keys();
    for step in 0..12 {
        let members = generation(step, size);
        for key in request_keys.iter().take(20) {
            let _ = policy.pick(&members, key.as_str());
        }
    }
}

/// Asserts an aged policy agrees with a fresh one on the same input.
///
/// Selections are compared by identity rather than index. Two policies handed
/// the same slice must pick the same member, and comparing indices would also
/// pass if both were wrong in the same way about ordering.
fn assert_matches_fresh<P, F>(label: &str, mut build: F, size: usize)
where
    F: FnMut() -> P,
    P: Policy<Candidate, str>,
{
    let mut aged = build();
    age(&mut aged, size);

    let members = generation(99, size);
    let request_keys = keys();

    // One reference for the whole sweep. It has never seen another membership,
    // which is the property that makes it a reference; rebuilding it per key
    // would only pay for the same table hundreds of times.
    let mut fresh = build();

    for key in &request_keys {
        let expected = describe(fresh.pick(&members, key.as_str()), &members);
        let actual = describe(aged.pick(&members, key.as_str()), &members);

        assert_eq!(
            actual, expected,
            "{label}: an aged policy disagreed with a fresh one on {key}"
        );
    }
}

/// Renders an outcome as the chosen identity, or the error.
fn describe(
    outcome: Result<Selection, PickError>,
    members: &[Candidate],
) -> Result<String, PickError> {
    outcome.map(|selection| members[selection.index()].id().clone())
}

#[test]
fn ring_hash_agrees_with_a_policy_that_never_cached() {
    for size in [4_usize, 17, 64] {
        assert_matches_fresh("ring hash", RingHash::<String>::default, size);
    }
}

#[test]
fn maglev_agrees_with_a_policy_that_never_cached() {
    for size in [4_usize, 17, 64] {
        assert_matches_fresh("maglev", Maglev::<String>::default, size);
    }
}

#[test]
fn bounded_load_agrees_with_a_policy_that_never_reused_scratch() {
    for size in [4_usize, 17, 64] {
        assert_matches_fresh(
            "bounded-load rendezvous",
            BoundedLoadRendezvous::default,
            size,
        );
    }
}

/// The churn the comparison relies on actually rebuilds the cached table.
///
/// Guards the guard. If every generation produced the same membership
/// fingerprint, the aged policy would never rebuild, the comparison above would
/// hold trivially, and it would be testing that a cache nobody invalidated
/// still matches itself.
#[test]
fn the_churn_used_above_really_does_rebuild() {
    let mut policy = RingHash::<String>::default();
    let request_keys = keys();
    let mut generations = Vec::new();

    for step in 0..12 {
        let members = generation(step, 17);
        let _ = policy.pick(&members, request_keys[0].as_str());
        generations.push(policy.generation());
    }

    let rebuilds = generations
        .windows(2)
        .filter(|pair| pair[0] != pair[1])
        .count();
    assert!(
        rebuilds >= 8,
        "the churn produced only {rebuilds} rebuilds across 12 generations, \
         so the equivalence law is barely exercised"
    );
}
