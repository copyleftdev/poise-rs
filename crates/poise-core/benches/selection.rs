#![allow(missing_docs)]
//! Steady-state selection cost for every stable policy.
//!
//! One group per policy, parameterized by candidate count, measuring a single
//! `pick` against an unchanged candidate set. This is the row-by-row
//! counterpart to the selection-cost table in the performance chapter: the
//! linear policies should track candidate count, and the cached ones should
//! stay flat once their table is built.
//!
//! Membership-change cost is deliberately absent here and measured separately,
//! because mixing the two would report an average of two different regimes and
//! describe neither.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use poise_core::{
    Policy,
    policy::{
        BoundedLoadRendezvous, LeastLoaded, LocalityWeightedRandom, Maglev, PowerOfTwoChoices,
        PriorityWeightedRandom, Random, Rendezvous, RingHash, RoundRobin, SmoothWeightedRoundRobin,
        WeightedRandom, WeightedRendezvous,
    },
};

#[path = "support.rs"]
mod support;

use support::{SIZES, candidates, keys, localized, prioritized};

/// Benchmarks a context-free policy across candidate counts.
fn bench_unkeyed<P, F>(criterion: &mut Criterion, name: &str, mut build: F)
where
    F: FnMut() -> P,
    P: Policy<support::Candidate>,
{
    let mut group = criterion.benchmark_group(name);
    for size in SIZES {
        let set = candidates(size);
        let mut policy = build();
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| black_box(policy.pick(black_box(&set), &()).unwrap()));
        });
    }
    group.finish();
}

/// Benchmarks a keyed policy across candidate counts, rotating the request key.
fn bench_keyed<P, F>(criterion: &mut Criterion, name: &str, mut build: F)
where
    F: FnMut() -> P,
    P: Policy<support::Candidate, str>,
{
    let request_keys = keys();
    let mut group = criterion.benchmark_group(name);
    for size in SIZES {
        let set = candidates(size);
        let mut policy = build();
        // Warm any cached table so the measured samples are steady state rather
        // than one rebuild averaged across many lookups.
        let _ = policy.pick(&set, request_keys[0].as_str());
        let mut cursor = 0_usize;
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| {
                cursor = (cursor + 1) % request_keys.len();
                let key = request_keys[cursor].as_str();
                black_box(policy.pick(black_box(&set), black_box(key)).unwrap())
            });
        });
    }
    group.finish();
}

fn selection(criterion: &mut Criterion) {
    bench_unkeyed(criterion, "select/round_robin", RoundRobin::new);
    bench_unkeyed(criterion, "select/smooth_weighted_round_robin", || {
        SmoothWeightedRoundRobin::<String>::new()
    });
    bench_unkeyed(criterion, "select/random", || Random::seeded(0x5eed));
    bench_unkeyed(criterion, "select/weighted_random", || {
        WeightedRandom::seeded(0x5eed)
    });
    bench_unkeyed(criterion, "select/least_loaded", LeastLoaded::new);
    bench_unkeyed(criterion, "select/power_of_two_choices", || {
        PowerOfTwoChoices::seeded(0x5eed)
    });

    bench_keyed(criterion, "select/rendezvous", Rendezvous::new);
    bench_keyed(
        criterion,
        "select/weighted_rendezvous",
        WeightedRendezvous::new,
    );
    bench_keyed(
        criterion,
        "select/bounded_load_rendezvous",
        BoundedLoadRendezvous::default,
    );
    bench_keyed(criterion, "select/ring_hash", RingHash::<String>::default);
    bench_keyed(criterion, "select/maglev", Maglev::<String>::default);

    // Priority and locality take wrapped candidates, so they do not fit the
    // helpers above.
    let mut group = criterion.benchmark_group("select/priority_weighted_random");
    for size in SIZES {
        let set = prioritized(size);
        let mut policy = PriorityWeightedRandom::seeded(0x5eed);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| black_box(policy.pick(black_box(&set), &()).unwrap()));
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("select/locality_weighted_random");
    for size in SIZES {
        let set = localized(size);
        let mut policy = LocalityWeightedRandom::seeded(0x5eed);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| black_box(policy.pick(black_box(&set), &()).unwrap()));
        });
    }
    group.finish();
}

criterion_group!(benches, selection);
criterion_main!(benches);
