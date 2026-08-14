#![allow(missing_docs)]
//! Membership-change cost for the policies that cache a table.
//!
//! `RingHash`, `Maglev`, and `SmoothWeightedRoundRobin` retain state keyed by
//! identity, and rebuild it when membership changes. Their steady-state lookup
//! is cheap by construction; the number worth knowing is what a rebuild costs,
//! since that is what a deployment pays during a rollout or an outage, and it
//! is the regime where a policy can surprise an operator.
//!
//! Each iteration alternates between two candidate sets differing by one
//! member, which forces a rebuild every time. That is a deliberate worst case:
//! real membership does not change on every request, so read these as the cost
//! of a change and not as a per-request cost.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use poise_core::{
    Policy,
    policy::{Maglev, RingHash, SmoothWeightedRoundRobin},
};

#[path = "support.rs"]
mod support;

use support::{SIZES, candidates, candidates_missing_one, keys};

/// Benchmarks a keyed policy while membership flips on every iteration.
fn bench_keyed_churn<P, F>(criterion: &mut Criterion, name: &str, mut build: F)
where
    F: FnMut() -> P,
    P: Policy<support::Candidate, str>,
{
    let request_keys = keys();
    let mut group = criterion.benchmark_group(name);
    for size in SIZES {
        let full = candidates(size);
        let reduced = candidates_missing_one(size);
        let mut policy = build();
        let _ = policy.pick(&full, request_keys[0].as_str());
        let mut cursor = 0_usize;
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| {
                cursor += 1;
                let set = if cursor % 2 == 0 { &full } else { &reduced };
                let key = request_keys[cursor % request_keys.len()].as_str();
                black_box(policy.pick(black_box(set), black_box(key)).unwrap())
            });
        });
    }
    group.finish();
}

fn membership(criterion: &mut Criterion) {
    bench_keyed_churn(criterion, "churn/ring_hash", RingHash::<String>::default);
    bench_keyed_churn(criterion, "churn/maglev", Maglev::<String>::default);

    let mut group = criterion.benchmark_group("churn/smooth_weighted_round_robin");
    for size in SIZES {
        let full = candidates(size);
        let reduced = candidates_missing_one(size);
        let mut policy = SmoothWeightedRoundRobin::<String>::new();
        let mut cursor = 0_usize;
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |bencher, _| {
            bencher.iter(|| {
                cursor += 1;
                let set = if cursor % 2 == 0 { &full } else { &reduced };
                black_box(policy.pick(black_box(set), &()).unwrap())
            });
        });
    }
    group.finish();
}

criterion_group!(benches, membership);
criterion_main!(benches);
