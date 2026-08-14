#![no_main]

use libfuzzer_sys::{
    arbitrary::{self, Arbitrary},
    fuzz_target,
};
use poise_core::{ProbeDecisionError, ProbeEntry, ProbePool, ProbePoolConfig, ProbeReading};
use std::{
    num::{NonZeroU32, NonZeroUsize},
    time::{Duration, Instant},
};

#[derive(Arbitrary, Debug)]
enum Action {
    Record {
        id: u8,
        requests_in_flight: u64,
        latency_micros: u32,
    },
    DecideFirst,
    DecideLowestLatency,
    DecideLowestRif,
    DecideNone,
    DecideOutOfBounds { overshoot: u8 },
    Advance { micros: u32 },
    Len,
    Clear,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    capacity: u8,
    max_uses: u8,
    max_age_micros: u32,
    actions: Vec<Action>,
}

fn lowest_latency(entries: &[ProbeEntry<u8>]) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .min_by_key(|(_, entry)| entry.latency())
        .map(|(index, _)| index)
}

fn lowest_rif(entries: &[ProbeEntry<u8>]) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .min_by_key(|(_, entry)| entry.requests_in_flight())
        .map(|(index, _)| index)
}

fuzz_target!(|scenario: Scenario| {
    let capacity = NonZeroUsize::new(usize::from(scenario.capacity).clamp(1, 64)).unwrap();
    let max_uses = NonZeroU32::new(u32::from(scenario.max_uses).clamp(1, 8)).unwrap();
    let max_age = Duration::from_micros(u64::from(scenario.max_age_micros).max(1));
    let Ok(config) = ProbePoolConfig::new(capacity, max_uses, max_age) else {
        return;
    };

    let pool: ProbePool<u8> = ProbePool::new(config);
    let base = Instant::now();
    let mut now = base;

    for action in scenario.actions {
        match action {
            Action::Record {
                id,
                requests_in_flight,
                latency_micros,
            } => {
                let reading =
                    ProbeReading::new(requests_in_flight, Duration::from_micros(latency_micros.into()));
                pool.record_at(id, reading, now);
                assert!(
                    pool.len_at(now) <= capacity.get(),
                    "recording exceeded the configured capacity"
                );
            }
            Action::DecideFirst => {
                check_decision(&pool, now, max_uses, max_age, |entries| {
                    (!entries.is_empty()).then_some(0)
                });
            }
            Action::DecideLowestLatency => {
                check_decision(&pool, now, max_uses, max_age, lowest_latency);
            }
            Action::DecideLowestRif => {
                check_decision(&pool, now, max_uses, max_age, lowest_rif);
            }
            Action::DecideNone => {
                let live = pool.len_at(now);
                let outcome = pool.decide_at(now, |_| None);
                let expected = if live == 0 {
                    ProbeDecisionError::NoProbes
                } else {
                    ProbeDecisionError::NoSelection
                };
                assert_eq!(outcome.err(), Some(expected));
                assert_eq!(
                    pool.len_at(now),
                    live,
                    "a rejected decision must not spend a reuse budget"
                );
            }
            Action::DecideOutOfBounds { overshoot } => {
                let live = pool.len_at(now);
                let outcome = pool.decide_at(now, |entries| {
                    Some(entries.len().saturating_add(usize::from(overshoot)))
                });
                let expected = if live == 0 {
                    ProbeDecisionError::NoProbes
                } else {
                    ProbeDecisionError::IndexOutOfBounds
                };
                assert_eq!(outcome.err(), Some(expected));
                assert_eq!(
                    pool.len_at(now),
                    live,
                    "an invalid index must not spend a reuse budget"
                );
            }
            Action::Advance { micros } => {
                now = now
                    .checked_add(Duration::from_micros(micros.into()))
                    .unwrap_or(now);
            }
            Action::Len => {
                assert!(pool.len_at(now) <= capacity.get());
            }
            Action::Clear => {
                pool.clear();
                assert_eq!(pool.len_at(now), 0);
                assert!(pool.is_empty_at(now));
            }
        }
    }
});

fn check_decision<F>(
    pool: &ProbePool<u8>,
    now: Instant,
    max_uses: NonZeroU32,
    max_age: Duration,
    choose: F,
) where
    F: FnOnce(&[ProbeEntry<u8>]) -> Option<usize>,
{
    let before = pool.len_at(now);
    match pool.decide_at(now, choose) {
        Ok(entry) => {
            assert!(before > 0, "a decision was produced from an empty pool");
            assert!(
                entry.age_at(now) < max_age,
                "an expired observation informed a decision"
            );
            assert!(
                entry.remaining_uses() < max_uses.get(),
                "a decision must charge at least one use"
            );
            let expected = if entry.remaining_uses() == 0 {
                before - 1
            } else {
                before
            };
            assert_eq!(
                pool.len_at(now),
                expected,
                "an exhausted entry is retired and a partially spent one is retained"
            );
        }
        Err(error) => assert_eq!(
            error,
            ProbeDecisionError::NoProbes,
            "a total selector only fails on an empty pool"
        ),
    }
}
