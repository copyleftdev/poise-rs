//! Exhaustive scheduler models for shared health state machines.

#![cfg(loom)]

use std::{
    num::{NonZeroU32, NonZeroUsize},
    time::Duration,
};

use loom::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use poise_core::Outcome;
use poise_health::{
    ActiveHealth, ActiveHealthConfig, ActiveStatus, CircuitConfig, CircuitSnapshot, OutcomeWindow,
    PassiveHealth,
};

#[test]
fn active_health_admits_only_one_overlapping_probe() {
    loom::model(|| {
        let config =
            ActiveHealthConfig::new(Duration::from_secs(1), NonZeroU32::MIN, NonZeroU32::MIN)
                .unwrap();
        let health = ActiveHealth::new(config);
        let inside = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let health = health.clone();
            let inside = Arc::clone(&inside);
            workers.push(thread::spawn(move || {
                if let Ok(probe) = health.try_start_probe() {
                    assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                    thread::yield_now();
                    assert_eq!(inside.fetch_sub(1, Ordering::SeqCst), 1);
                    probe.cancel();
                }
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(inside.load(Ordering::SeqCst), 0);
        assert!(!health.snapshot().probe_in_flight());
    });
}

#[test]
fn forcing_status_invalidates_every_interleaving_of_a_stale_probe() {
    loom::model(|| {
        let config =
            ActiveHealthConfig::new(Duration::from_secs(1), NonZeroU32::MIN, NonZeroU32::MIN)
                .unwrap();
        let health = ActiveHealth::new(config);
        let stale = health.try_start_probe().unwrap();

        let forced = health.clone();
        let force_worker = thread::spawn(move || forced.force_status(ActiveStatus::Healthy));
        let result_worker = thread::spawn(move || stale.unhealthy());
        force_worker.join().unwrap();
        result_worker.join().unwrap();

        let snapshot = health.snapshot();
        assert_eq!(snapshot.status(), ActiveStatus::Healthy);
        assert!(!snapshot.probe_in_flight());
        assert_eq!(snapshot.consecutive_healthy(), 0);
        assert_eq!(snapshot.consecutive_unhealthy(), 0);
    });
}

#[test]
fn concurrent_circuit_failures_are_never_lost() {
    loom::model(|| {
        let config =
            CircuitConfig::new(NonZeroU32::new(2).unwrap(), Duration::from_secs(60)).unwrap();
        let health = PassiveHealth::new(config);
        let first = health.try_acquire().unwrap();
        let second = health.try_acquire().unwrap();

        let first_worker = thread::spawn(move || first.failure());
        let second_worker = thread::spawn(move || second.overloaded());
        first_worker.join().unwrap();
        second_worker.join().unwrap();

        assert!(matches!(health.snapshot(), CircuitSnapshot::Open { .. }));
    });
}

#[test]
fn rolling_window_snapshot_is_coherent_under_record_and_clear() {
    loom::model(|| {
        let window = OutcomeWindow::new(NonZeroUsize::new(2).unwrap());
        let recorder = window.clone();
        let clearer = window.clone();

        let first = thread::spawn(move || {
            recorder.record(Outcome::Failure);
            recorder.record(Outcome::Overloaded);
        });
        let second = thread::spawn(move || {
            clearer.clear();
            clearer.record(Outcome::Success);
        });
        first.join().unwrap();
        second.join().unwrap();

        let stats = window.stats();
        assert!(stats.samples() <= 2);
        assert_eq!(
            stats.samples(),
            stats.successes() + stats.failures() + stats.overloaded()
        );
        assert!(stats.penalty().get().is_finite());
        assert!(stats.penalty().get() >= 0.0);
    });
}
