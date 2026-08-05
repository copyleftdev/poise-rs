//! Observable public contracts discovered and enforced through mutation testing.

use std::{
    hash::{BuildHasher, Hasher},
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    time::Duration,
};

use poise_core::{
    AtCapacity, Backend, Candidate, Fnv1a64, InvalidWeight, Outcome, PeakEwma, PeakEwmaConfigError,
    PickError, Policy, Status, Weight,
    policy::{
        BoundedLoadConfig, BoundedLoadRendezvous, LocalityWeightedRandom, Localized, Maglev,
        MaglevConfig, PanicMode, Prioritized, PriorityCandidate, PriorityConfig, PriorityMode,
        PriorityWeightedRandom, Rendezvous, RingHash, RingHashConfig, RoundRobin,
        SmoothWeightedRoundRobin,
    },
};

#[test]
fn backend_mutators_change_only_the_requested_field() {
    let mut backend = Backend::new("api")
        .with_data(7_u8)
        .with_load(11_u64)
        .with_weight(Weight::new(2).unwrap());
    backend.set_load(13);
    backend.set_weight(Weight::new(3).unwrap());
    backend.set_status(Status::Unavailable);

    assert_eq!(backend.id(), &"api");
    assert_eq!(backend.data(), &7);
    assert_eq!(backend.load(), &13);
    assert_eq!(backend.weight().get(), 3);
    assert_eq!(backend.status(), Status::Unavailable);
}

#[test]
fn nonzero_weight_conversion_preserves_the_value() {
    let value = NonZeroU32::new(37).unwrap();
    assert_eq!(Weight::from(value).get(), 37);
    assert_eq!(NonZeroU32::from(Weight::from(value)), value);
}

#[test]
fn every_outcome_has_an_explicit_failure_and_observation_classification() {
    assert!(!Outcome::Success.is_failure());
    assert!(Outcome::Success.is_observation());
    assert!(Outcome::Failure.is_failure());
    assert!(Outcome::Failure.is_observation());
    assert!(Outcome::Overloaded.is_failure());
    assert!(Outcome::Overloaded.is_observation());
    assert!(!Outcome::Cancelled.is_failure());
    assert!(!Outcome::Cancelled.is_observation());
}

#[test]
fn fnv_typed_writes_match_explicit_little_endian_bytes() {
    macro_rules! assert_typed_write {
        ($method:ident, $value:expr) => {{
            let value = $value;
            let mut typed = Fnv1a64::default();
            typed.$method(value);
            let mut bytes = Fnv1a64::default();
            bytes.write(&value.to_le_bytes());
            assert_eq!(typed.finish(), bytes.finish(), stringify!($method));
        }};
    }

    assert_typed_write!(write_u8, 0xa5_u8);
    assert_typed_write!(write_u16, 0xa5c3_u16);
    assert_typed_write!(write_u32, 0xa5c3_91e7_u32);
    assert_typed_write!(write_u64, 0xa5c3_91e7_4268_f0bd_u64);
    assert_typed_write!(write_u128, 0xa5c3_91e7_4268_f0bd_1029_3847_56ab_cdef_u128);
    assert_typed_write!(write_usize, usize::MAX / 3);
    assert_typed_write!(write_i8, -37_i8);
    assert_typed_write!(write_i16, -12_345_i16);
    assert_typed_write!(write_i32, -123_456_789_i32);
    assert_typed_write!(write_i64, -1_234_567_890_123_456_i64);
    assert_typed_write!(write_i128, -12_345_678_901_234_567_890_i128);
    assert_typed_write!(write_isize, isize::MIN / 3);
}

#[test]
fn load_tracker_accessors_raii_and_debug_output_are_observable() {
    let limit = NonZeroU64::new(2).unwrap();
    let tracker = poise_core::InFlight::with_limit(limit);
    assert_eq!(tracker.limit(), Some(limit));
    assert!(format!("{tracker:?}").contains("limit: Some(2)"));

    let guard = tracker.start().unwrap();
    assert!(format!("{guard:?}").starts_with("InFlightGuard"));
    drop(guard);
    assert_eq!(tracker.current(), 0);

    let first = tracker.start().unwrap();
    let second = tracker.start().unwrap();
    let error = tracker.start().unwrap_err();
    assert_eq!(error.limit(), Some(limit));
    assert_eq!(
        error.to_string(),
        "the in-flight limit of 2 has been reached"
    );
    drop((first, second));
}

#[test]
fn peak_ewma_configuration_score_and_guard_lifetime_are_observable() {
    let default_rtt = Duration::from_millis(100);
    let half_life = Duration::from_secs(60);
    let tracker = PeakEwma::new(default_rtt, half_life).unwrap();
    assert_eq!(tracker.default_rtt(), default_rtt);
    assert_eq!(tracker.half_life(), half_life);
    assert!(format!("{tracker:?}").starts_with("PeakEwma"));

    let guard = tracker.start().unwrap();
    assert!(format!("{guard:?}").starts_with("PeakEwmaGuard"));
    let score = tracker.score().get();
    assert!((0.19..=0.21).contains(&score), "score={score}");
    drop(guard);
    assert_eq!(tracker.in_flight(), 0);
}

#[test]
fn public_error_messages_are_nonempty_and_specific() {
    let pick_errors = [
        PickError::Empty,
        PickError::NoEligibleCandidates,
        PickError::WeightOverflow,
        PickError::DuplicateIdentity,
        PickError::StateCapacityExceeded,
        PickError::LoadOverflow,
        PickError::PanicRejected,
        PickError::InconsistentTopology,
    ];
    for error in pick_errors {
        assert!(!error.to_string().is_empty());
    }

    let invalid_weight: InvalidWeight = Weight::new(0).unwrap_err();
    assert_eq!(
        invalid_weight.to_string(),
        "a backend weight must be greater than zero"
    );
    assert_eq!(
        PeakEwma::new(Duration::ZERO, Duration::from_secs(1))
            .unwrap_err()
            .to_string(),
        "peak EWMA default RTT must be non-zero"
    );
    assert_eq!(
        PeakEwma::new(Duration::from_millis(1), Duration::ZERO)
            .unwrap_err()
            .to_string(),
        "peak EWMA half-life must be non-zero"
    );
    assert!(
        !BoundedLoadConfig::new(99)
            .unwrap_err()
            .to_string()
            .is_empty()
    );
    assert!(
        !PriorityConfig::new(99, 50, PanicMode::default())
            .unwrap_err()
            .to_string()
            .is_empty()
    );
    assert!(
        !RingHashConfig::new(NonZeroU32::new(2).unwrap(), NonZeroUsize::new(1).unwrap(),)
            .unwrap_err()
            .to_string()
            .is_empty()
    );
    assert!(!MaglevConfig::new(4).unwrap_err().to_string().is_empty());
}

#[test]
fn bounded_load_decision_reports_exact_load_capacity_and_scratch() {
    let singleton = [Backend::new("only").with_load(3_u64)];
    let mut policy = BoundedLoadRendezvous::new(BoundedLoadConfig::new(100).unwrap());
    let decision = policy.decide(&singleton, &7_u64).unwrap();
    assert!(!decision.spilled());
    assert_eq!(decision.selected_load(), 3);
    assert_eq!(decision.selected_capacity(), 4);

    let many = [
        Backend::new("first").with_load(3_u64),
        Backend::new("second").with_load(7_u64),
        Backend::new("third").with_load(11_u64),
    ];
    policy.decide(&many, &7_u64).unwrap();
    assert!(policy.scratch_capacity() >= many.len());
}

#[derive(Clone, Copy, Debug)]
struct ExplicitTopologyCandidate {
    status: Status,
    member: bool,
    panic: bool,
}

#[derive(Clone, Copy, Debug)]
struct DefaultTopologyCandidate {
    status: Status,
}

impl Candidate for DefaultTopologyCandidate {
    type Id = u8;
    type Load = ();

    fn id(&self) -> &Self::Id {
        &0
    }

    fn load(&self) -> &Self::Load {
        &()
    }

    fn status(&self) -> Status {
        self.status
    }
}

impl PriorityCandidate for DefaultTopologyCandidate {
    fn priority(&self) -> u32 {
        0
    }
}

impl Candidate for ExplicitTopologyCandidate {
    type Id = u8;
    type Load = ();

    fn id(&self) -> &Self::Id {
        &0
    }

    fn load(&self) -> &Self::Load {
        &()
    }

    fn status(&self) -> Status {
        self.status
    }
}

impl PriorityCandidate for ExplicitTopologyCandidate {
    fn priority(&self) -> u32 {
        0
    }

    fn is_priority_member(&self) -> bool {
        self.member
    }

    fn is_panic_eligible(&self) -> bool {
        self.panic
    }
}

#[test]
fn topology_wrappers_delegate_explicit_membership_and_panic_exclusions() {
    let excluded = ExplicitTopologyCandidate {
        status: Status::Ready,
        member: false,
        panic: false,
    };
    let localized = Localized::new(excluded, "west");
    assert!(!localized.is_priority_member());
    assert!(!localized.is_panic_eligible());

    let prioritized = Prioritized::new(Backend::new("api"), 0).with_panic_eligibility(false);
    assert!(!prioritized.allows_panic());
    assert!(Prioritized::new(Backend::new("api"), 0).allows_panic());

    assert!(
        DefaultTopologyCandidate {
            status: Status::Ready
        }
        .is_panic_eligible()
    );
    assert!(
        !DefaultTopologyCandidate {
            status: Status::Draining
        }
        .is_panic_eligible()
    );
}

#[test]
fn exact_panic_threshold_remains_healthy_and_global_panic_samples_every_member() {
    let threshold = [
        Prioritized::new(Backend::new("ready"), 0),
        Prioritized::new(Backend::new("down").with_status(Status::Unavailable), 0),
    ];
    let config = PriorityConfig::new(100, 50, PanicMode::UseAll).unwrap();
    let decision = PriorityWeightedRandom::seeded_with(config, 4)
        .decide(&threshold)
        .unwrap();
    assert_eq!(decision.mode(), PriorityMode::Healthy);
    assert_eq!(decision.selection().index(), 0);

    let panic = [
        Prioritized::new(Backend::new("down-a").with_status(Status::Unavailable), 0),
        Prioritized::new(Backend::new("down-b").with_status(Status::Unavailable), 1),
    ];
    let mut policy = PriorityWeightedRandom::seeded_with(config, 9);
    let mut seen = [false; 2];
    for _ in 0..100 {
        seen[policy.decide(&panic).unwrap().selection().index()] = true;
    }
    assert!(seen.into_iter().all(|selected| selected));
}

#[test]
fn locality_rounding_keeps_tiny_healthy_localities_and_excludes_empty_ones() {
    let config = PriorityConfig::new(100, 0, PanicMode::UseAll).unwrap();
    let huge = Weight::new(2_000_000).unwrap();
    let locality_boost = Weight::new(u32::MAX).unwrap();
    let candidates = [
        Localized::new(Prioritized::new(Backend::new("tiny"), 0), "tiny")
            .with_locality_weight(locality_boost),
        Localized::new(
            Prioritized::new(
                Backend::new("tiny-down")
                    .with_weight(huge)
                    .with_status(Status::Unavailable),
                0,
            ),
            "tiny",
        )
        .with_locality_weight(locality_boost),
        Localized::new(
            Prioritized::new(Backend::new("healthy").with_weight(huge), 0),
            "healthy",
        ),
        Localized::new(
            Prioritized::new(
                Backend::new("empty")
                    .with_weight(huge)
                    .with_status(Status::Unavailable),
                0,
            ),
            "empty",
        )
        .with_locality_weight(locality_boost),
    ];
    let mut policy = LocalityWeightedRandom::seeded_with(config, 17);
    for _ in 0..100 {
        let index = policy.decide(&candidates).unwrap().selection().index();
        assert_eq!(index, 0, "the boosted tiny locality should dominate");
    }
}

#[test]
fn locality_ticket_boundary_can_reach_the_second_unit_weight_locality() {
    let config = PriorityConfig::new(100, 0, PanicMode::UseAll).unwrap();
    let down_weight = Weight::new(999_999).unwrap();
    let candidates = [
        Localized::new(Prioritized::new(Backend::new("a"), 0), "a"),
        Localized::new(
            Prioritized::new(
                Backend::new("a-down")
                    .with_weight(down_weight)
                    .with_status(Status::Unavailable),
                0,
            ),
            "a",
        ),
        Localized::new(Prioritized::new(Backend::new("b"), 0), "b"),
        Localized::new(
            Prioritized::new(
                Backend::new("b-down")
                    .with_weight(down_weight)
                    .with_status(Status::Unavailable),
                0,
            ),
            "b",
        ),
    ];
    let mut policy = LocalityWeightedRandom::seeded_with(config, 23);
    let mut seen = [false; 2];
    for _ in 0..100 {
        let index = policy.decide(&candidates).unwrap().selection().index();
        seen[usize::from(index >= 2)] = true;
    }
    assert!(seen.into_iter().all(|selected| selected));
}

#[test]
fn round_robin_normalizes_an_arbitrary_initial_cursor_before_scanning() {
    let candidates = [Backend::new("a"), Backend::new("b"), Backend::new("c")];
    let mut policy = RoundRobin::with_cursor(usize::MAX);
    assert_eq!(
        policy.pick(&candidates, &()).unwrap().index(),
        usize::MAX % 3
    );
}

#[test]
fn topology_decisions_report_exact_effective_weight_and_real_scratch_capacity() {
    let candidates = [
        Localized::new(Prioritized::new(Backend::new("api"), 0), "west")
            .with_locality_weight(Weight::new(3).unwrap()),
    ];
    let decision = LocalityWeightedRandom::seeded(1)
        .decide(&candidates)
        .unwrap();
    assert_eq!(decision.effective_locality_weight(), 3_000_000);

    let many_priority: Vec<_> = (0..64)
        .map(|id| Prioritized::new(Backend::new(id), id % 4))
        .collect();
    let mut priority = PriorityWeightedRandom::seeded(2);
    priority.decide(&many_priority).unwrap();
    let (members, groups) = priority.scratch_capacity();
    assert!(members >= many_priority.len());
    assert!(groups >= many_priority.len());

    let many_locality: Vec<_> = many_priority
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| Localized::new(candidate, index % 8))
        .collect();
    let mut locality = LocalityWeightedRandom::seeded(3);
    locality.decide(&many_locality).unwrap();
    let ((members, groups), localities) = locality.scratch_capacity();
    assert!(members >= many_locality.len());
    assert!(groups >= many_locality.len());
    assert!(localities >= many_locality.len());
}

#[test]
fn cached_affinity_policies_and_swrr_reset_all_observable_state() {
    let candidates = [Backend::new(1_u32), Backend::new(2_u32)];

    let mut maglev = Maglev::<u32>::new(MaglevConfig::new(17).unwrap());
    maglev.pick(&candidates, &9_u64).unwrap();
    assert_eq!(maglev.generation(), 1);
    maglev.reset();
    assert_eq!(maglev.generation(), 0);
    assert_eq!(maglev.member_count(), 0);
    assert_eq!(maglev.populated_table_size(), 0);

    let mut ring = RingHash::<u32>::new(
        RingHashConfig::new(NonZeroU32::new(2).unwrap(), NonZeroUsize::new(64).unwrap()).unwrap(),
    );
    ring.pick(&candidates, &9_u64).unwrap();
    assert_eq!(ring.generation(), 1);
    ring.reset();
    assert_eq!(ring.generation(), 0);
    assert_eq!(ring.member_count(), 0);
    assert_eq!(ring.virtual_node_count(), 0);

    let mut swrr = SmoothWeightedRoundRobin::new();
    swrr.pick(&candidates, &()).unwrap();
    assert_eq!(swrr.tracked_len(), candidates.len());
    swrr.reset();
    assert_eq!(swrr.tracked_len(), 0);
}

#[derive(Clone, Copy, Debug, Default)]
struct ConstantBuildHasher;

impl BuildHasher for ConstantBuildHasher {
    type Hasher = ConstantHasher;

    fn build_hasher(&self) -> Self::Hasher {
        ConstantHasher
    }
}

#[derive(Clone, Copy, Debug)]
struct ConstantHasher;

impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        7
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

#[test]
fn rendezvous_hash_ties_stably_prefer_the_first_candidate() {
    let candidates = [Backend::new("first"), Backend::new("second")];
    let mut policy = Rendezvous::with_hasher(ConstantBuildHasher);
    assert_eq!(policy.pick(&candidates, &"key").unwrap().index(), 0);
}

#[test]
fn peak_ewma_error_variants_remain_distinguishable() {
    assert_ne!(
        PeakEwmaConfigError::ZeroDefaultRtt,
        PeakEwmaConfigError::ZeroHalfLife
    );
    let _ = std::mem::size_of::<AtCapacity>();
}
