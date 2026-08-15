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

/// Sequential reference for the pool's observable contract.
///
/// Deliberately naive and independent of the implementation: a vector, a use
/// counter, and no locking. Its only job is to say what a *single-threaded*
/// caller would have observed, which is the standard every concurrent history
/// below is held to.
///
/// Time is absent because these models run every operation at one instant under
/// a lifetime that never expires, and selection always takes index zero, which
/// is the selector the concurrent threads pass.
#[derive(Clone, Debug)]
struct Model {
    max_uses: u32,
    entries: Vec<(&'static str, u32)>,
}

/// What a thread can call.
#[derive(Clone, Copy, Debug)]
enum Call {
    Decide,
    Record(&'static str),
}

/// What a thread observed, in enough detail to distinguish a lost update.
///
/// Carrying `remaining` is the point. A history where two selectors both report
/// one use left has spent the budget once and charged it twice, and a test that
/// counted successes alone would accept it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Selected { id: &'static str, remaining: u32 },
    Rejected(ProbeDecisionError),
    Recorded,
}

impl Model {
    fn new(max_uses: u32, seeded: &[&'static str]) -> Self {
        Self {
            max_uses,
            entries: seeded.iter().map(|id| (*id, max_uses)).collect(),
        }
    }

    fn apply(&mut self, call: Call) -> Outcome {
        match call {
            Call::Record(id) => {
                self.entries.push((id, self.max_uses));
                Outcome::Recorded
            }
            Call::Decide => {
                if self.entries.is_empty() {
                    return Outcome::Rejected(ProbeDecisionError::NoProbes);
                }
                let entry = &mut self.entries[0];
                entry.1 -= 1;
                let observed = Outcome::Selected {
                    id: entry.0,
                    remaining: entry.1,
                };
                if entry.1 == 0 {
                    self.entries.remove(0);
                }
                observed
            }
        }
    }
}

/// Whether some sequential order of the observed operations explains them all.
///
/// Each thread's operations must stay in program order; the search is over the
/// interleavings of those sequences. A history is linearizable when at least
/// one such order, replayed against the reference, reproduces every observation
/// exactly. Exhaustive because these models issue at most a handful of
/// operations.
fn linearizable(model: &Model, threads: &[Vec<(Call, Outcome)>], progress: &mut [usize]) -> bool {
    if threads
        .iter()
        .zip(progress.iter())
        .all(|(thread, done)| *done == thread.len())
    {
        return true;
    }

    for index in 0..threads.len() {
        let done = progress[index];
        if done == threads[index].len() {
            continue;
        }
        let (call, observed) = threads[index][done];
        let mut next = model.clone();
        if next.apply(call) == observed {
            progress[index] += 1;
            if linearizable(&next, threads, progress) {
                return true;
            }
            progress[index] -= 1;
        }
    }

    false
}

fn assert_linearizable(model: &Model, threads: &[Vec<(Call, Outcome)>]) {
    let mut progress = vec![0; threads.len()];
    assert!(
        linearizable(model, threads, &mut progress),
        "no sequential order explains {threads:?}"
    );
}

fn observe(result: Result<poise_core::ProbeEntry<&'static str>, ProbeDecisionError>) -> Outcome {
    match result {
        Ok(entry) => Outcome::Selected {
            id: entry.id(),
            remaining: entry.remaining_uses(),
        },
        Err(error) => Outcome::Rejected(error),
    }
}

/// Concurrent consumption of a reusable observation is linearizable.
///
/// The counting invariant elsewhere in this file asks whether the budget was
/// overspent. This asks the stronger question: whether what the selectors saw
/// could have happened at all. Two selectors both reporting one use remaining
/// spend two charges against one decrement -- a lost update -- while still
/// totalling two successes against a budget of two, so only the ordering
/// question rejects it.
#[test]
fn concurrent_consumption_is_linearizable() {
    loom::model(|| {
        let pool = ProbePool::new(config(4, 2));
        let now = Instant::now();
        pool.record_at("shared", reading(), now);

        let workers: Vec<_> = (0..2)
            .map(|_| {
                let pool = pool.clone();
                thread::spawn(move || {
                    observe(pool.decide_at(now, |entries| (!entries.is_empty()).then_some(0)))
                })
            })
            .collect();

        let threads: Vec<Vec<(Call, Outcome)>> = workers
            .into_iter()
            .map(|worker| vec![(Call::Decide, worker.join().unwrap())])
            .collect();

        assert_linearizable(&Model::new(2, &["shared"]), &threads);
    });
}

/// Exhausting a budget across more selectors than uses is linearizable.
///
/// Three selectors against two uses must observe one use left, then none, then
/// an empty pool, in some order. Any other combination -- two rejections, or
/// three successes -- has no sequential explanation.
#[test]
fn budget_exhaustion_is_linearizable() {
    loom::model(|| {
        let pool = ProbePool::new(config(4, 2));
        let now = Instant::now();
        pool.record_at("shared", reading(), now);

        let workers: Vec<_> = (0..3)
            .map(|_| {
                let pool = pool.clone();
                thread::spawn(move || {
                    observe(pool.decide_at(now, |entries| (!entries.is_empty()).then_some(0)))
                })
            })
            .collect();

        let threads: Vec<Vec<(Call, Outcome)>> = workers
            .into_iter()
            .map(|worker| vec![(Call::Decide, worker.join().unwrap())])
            .collect();

        assert_linearizable(&Model::new(2, &["shared"]), &threads);
    });
}

/// A recording racing a selection is linearizable.
///
/// The selector may or may not see the arriving observation depending on the
/// order, and both are legitimate. What would not be is observing a half-built
/// entry: an identity paired with a budget that no single ordering produces.
#[test]
fn recording_against_selection_is_linearizable() {
    loom::model(|| {
        let pool = ProbePool::new(config(4, 1));
        let now = Instant::now();
        pool.record_at("early", reading(), now);

        let selector = {
            let pool = pool.clone();
            thread::spawn(move || {
                observe(pool.decide_at(now, |entries| (!entries.is_empty()).then_some(0)))
            })
        };
        let recorder = {
            let pool = pool.clone();
            thread::spawn(move || {
                pool.record_at("late", reading(), now);
                Outcome::Recorded
            })
        };

        let selected = selector.join().unwrap();
        let arrival = recorder.join().unwrap();

        assert_linearizable(
            &Model::new(1, &["early"]),
            &[
                vec![(Call::Decide, selected)],
                vec![(Call::Record("late"), arrival)],
            ],
        );
    });
}

/// The linearizability check rejects a lost update.
///
/// Guards the guard. Two selectors both reporting one use left is exactly the
/// bug the ordering question exists to catch: two charges landing on one
/// decrement. It totals two successes against a budget of two, so a counting
/// invariant accepts it, and no sequential order produces it.
///
/// Without this, a checker that always answered yes would pass every model
/// above and prove nothing.
#[test]
fn the_linearizability_check_rejects_a_lost_update() {
    let lost_update = vec![
        vec![(
            Call::Decide,
            Outcome::Selected {
                id: "shared",
                remaining: 1,
            },
        )],
        vec![(
            Call::Decide,
            Outcome::Selected {
                id: "shared",
                remaining: 1,
            },
        )],
    ];
    let mut progress = vec![0; lost_update.len()];
    assert!(
        !linearizable(&Model::new(2, &["shared"]), &lost_update, &mut progress),
        "the check accepted a history no sequential order can produce"
    );
}

/// The linearizability check accepts the history that should be legal.
///
/// The other half of guarding the guard: a check that rejected everything
/// would also pass the test above while making the models meaningless.
#[test]
fn the_linearizability_check_accepts_a_valid_interleaving() {
    let valid = vec![
        vec![(
            Call::Decide,
            Outcome::Selected {
                id: "shared",
                remaining: 1,
            },
        )],
        vec![(
            Call::Decide,
            Outcome::Selected {
                id: "shared",
                remaining: 0,
            },
        )],
    ];
    let mut progress = vec![0; valid.len()];
    assert!(
        linearizable(&Model::new(2, &["shared"]), &valid, &mut progress),
        "the check rejected a history a single-threaded caller could produce"
    );
}
