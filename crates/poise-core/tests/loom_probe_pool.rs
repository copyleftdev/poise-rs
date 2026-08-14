//! Exhaustive scheduler models for shared probe-pool consumption.
//!
//! The reuse bound is what separates a probe pool from naive stale-information
//! balancing. If two selectors can both read one observation reporting an idle
//! replica, both route to it, and the observation has caused exactly the
//! imbalance it was collected to prevent. These models explore every
//! interleaving of the read-and-charge sequence.

#![cfg(loom)]

use std::{
    num::{NonZeroU32, NonZeroUsize},
    time::{Duration, Instant},
};

use loom::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use poise_core::{ProbeDecisionError, ProbePool, ProbePoolConfig, ProbeReading};

const NEVER_EXPIRES: Duration = Duration::from_secs(86_400);

fn config(capacity: usize, max_uses: u32) -> ProbePoolConfig {
    ProbePoolConfig::new(
        NonZeroUsize::new(capacity).unwrap(),
        NonZeroU32::new(max_uses).unwrap(),
        NEVER_EXPIRES,
    )
    .unwrap()
}

fn reading() -> ProbeReading {
    ProbeReading::new(0, Duration::from_millis(1))
}

#[test]
fn one_observation_informs_one_decision_under_concurrent_selection() {
    loom::model(|| {
        let pool = ProbePool::new(config(4, 1));
        let now = Instant::now();
        pool.record_at("idle", reading(), now);

        let decisions = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let pool = pool.clone();
            let decisions = Arc::clone(&decisions);
            workers.push(thread::spawn(move || {
                match pool.decide_at(now, |entries| (!entries.is_empty()).then_some(0)) {
                    Ok(entry) => {
                        assert_eq!(entry.id(), &"idle");
                        assert_eq!(entry.remaining_uses(), 0);
                        decisions.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(error) => assert_eq!(error, ProbeDecisionError::NoProbes),
                }
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(
            decisions.load(Ordering::SeqCst),
            1,
            "a single-use observation must not steer two selectors"
        );
        assert_eq!(pool.len_at(now), 0);
    });
}

#[test]
fn a_reusable_observation_is_never_overspent() {
    loom::model(|| {
        let pool = ProbePool::new(config(4, 2));
        let now = Instant::now();
        pool.record_at("shared", reading(), now);

        let decisions = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let pool = pool.clone();
            let decisions = Arc::clone(&decisions);
            workers.push(thread::spawn(move || {
                if pool
                    .decide_at(now, |entries| (!entries.is_empty()).then_some(0))
                    .is_ok()
                {
                    decisions.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(
            decisions.load(Ordering::SeqCst),
            2,
            "both uses of a two-use observation must be spendable"
        );
        assert_eq!(pool.len_at(now), 0, "the exhausted observation is retired");
    });
}

#[test]
fn concurrent_recording_never_exceeds_capacity() {
    loom::model(|| {
        let pool = ProbePool::new(config(1, 1));
        let now = Instant::now();
        let mut workers = Vec::new();

        for id in ["first", "second"] {
            let pool = pool.clone();
            workers.push(thread::spawn(move || {
                pool.record_at(id, reading(), now);
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(pool.len_at(now), 1, "capacity bounds concurrent recording");
    });
}

#[test]
fn recording_races_with_selection_without_losing_the_bound() {
    loom::model(|| {
        let pool = ProbePool::new(config(1, 1));
        let now = Instant::now();
        pool.record_at("initial", reading(), now);

        let selector = {
            let pool = pool.clone();
            thread::spawn(move || {
                pool.decide_at(now, |entries| (!entries.is_empty()).then_some(0))
                    .is_ok()
            })
        };
        let recorder = {
            let pool = pool.clone();
            thread::spawn(move || pool.record_at("late", reading(), now))
        };

        selector.join().unwrap();
        recorder.join().unwrap();

        assert!(
            pool.len_at(now) <= 1,
            "capacity holds however the two operations interleave"
        );
    });
}
