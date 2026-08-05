use poise_core::{Backend, Candidate, PickError, Selection, Status, Weight};
use proptest::{
    collection::vec,
    prelude::*,
    test_runner::{FileFailurePersistence, TestCaseResult},
};

#[allow(dead_code)]
pub type TestBackend = Backend<u32, (), u64>;

#[derive(Clone, Copy, Debug)]
pub struct CandidateSpec {
    pub weight: u32,
    pub load: u64,
    pub status: Status,
}

prop_compose! {
    pub fn candidate_specs()
        (raw in vec((1_u32..=20, 0_u64..=10_000, 0_u8..3), 0..16))
        -> Vec<CandidateSpec>
    {
        raw.into_iter()
            .map(|(weight, load, status)| CandidateSpec {
                weight,
                load,
                status: status_from_byte(status),
            })
            .collect()
    }
}

pub fn property_config() -> ProptestConfig {
    let mut config =
        ProptestConfig::with_failure_persistence(FileFailurePersistence::WithSource("regressions"));
    config.cases = 256;
    if let Ok(cases) = std::env::var("POISE_PROPTEST_CASES") {
        config.cases = cases
            .parse()
            .expect("POISE_PROPTEST_CASES must be an unsigned 32-bit integer");
    }
    config.max_shrink_iters = 10_000;
    config
}

#[allow(dead_code)]
pub fn backends(specs: &[CandidateSpec]) -> Vec<TestBackend> {
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            Backend::new(u32::try_from(index).expect("property vectors are small"))
                .with_load(spec.load)
                .with_weight(
                    Weight::new(spec.weight).expect("strategies generate positive weights"),
                )
                .with_status(spec.status)
        })
        .collect()
}

#[allow(dead_code)]
pub fn assert_valid_selection(
    result: Result<Selection, PickError>,
    candidates: &[TestBackend],
) -> TestCaseResult {
    match result {
        Ok(selection) => {
            prop_assert!(selection.index() < candidates.len());
            prop_assert!(candidates[selection.index()].is_eligible());
        }
        Err(error) => {
            let expected = if candidates.is_empty() {
                PickError::Empty
            } else {
                PickError::NoEligibleCandidates
            };
            prop_assert_eq!(error, expected);
            prop_assert!(!candidates.iter().any(Candidate::is_eligible));
        }
    }
    Ok(())
}

const fn status_from_byte(value: u8) -> Status {
    match value {
        0 => Status::Ready,
        1 => Status::Draining,
        _ => Status::Unavailable,
    }
}
