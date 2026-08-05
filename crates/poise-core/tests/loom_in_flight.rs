//! Exhaustive scheduler models for the lock-free in-flight counter.

#![cfg(loom)]

use std::num::NonZeroU64;

use loom::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use poise_core::InFlight;

#[test]
fn limit_one_never_admits_two_overlapping_guards() {
    loom::model(|| {
        let tracker = InFlight::with_limit(NonZeroU64::MIN);
        let inside = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let tracker = tracker.clone();
            let inside = Arc::clone(&inside);
            workers.push(thread::spawn(move || {
                if let Ok(guard) = tracker.start() {
                    assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                    thread::yield_now();
                    assert_eq!(inside.fetch_sub(1, Ordering::SeqCst), 1);
                    drop(guard);
                }
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(inside.load(Ordering::SeqCst), 0);
        assert_eq!(tracker.current(), 0);
    });
}

#[test]
fn concurrent_unbounded_guards_balance_exactly() {
    loom::model(|| {
        let tracker = InFlight::new();
        let mut workers = Vec::new();

        for _ in 0..2 {
            let tracker = tracker.clone();
            workers.push(thread::spawn(move || {
                let guard = tracker.start().unwrap();
                thread::yield_now();
                drop(guard);
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(tracker.current(), 0);
    });
}
