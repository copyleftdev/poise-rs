//! Shared fixtures for the selection and membership benchmarks.
//!
//! Included rather than imported: a `benches/` file is its own crate root, and
//! two harnesses need the same candidate shapes.
//!
//! Everything here is `pub(crate)`. These are fixtures, not API, and the
//! workspace's API lints are aimed at exported items.

// Each harness includes this module and uses the subset of fixtures it needs.
#![allow(dead_code)]

use poise_core::{
    Backend, Status, Weight,
    policy::{Localized, Prioritized},
};

/// Candidate counts spanning the range the performance model describes.
///
/// Small is a single rack, medium a typical service, large the point where an
/// `O(n)` scan per selection stops being free. Anything below the largest here
/// hides the difference between the linear and cached policies, which is most
/// of what these benchmarks exist to show.
pub(crate) const SIZES: [usize; 3] = [8, 64, 512];

/// The load metric attached to every candidate.
///
/// Load is what the load-aware policies read; the affinity policies ignore it.
/// One candidate shape serves both so a size parameter means the same thing
/// across every group.
pub(crate) type Load = u64;

/// A candidate carrying identity, weight, status, and load.
pub(crate) type Candidate = Backend<String, (), Load>;

/// Builds `count` eligible candidates with spread weights and loads.
///
/// Weights cycle through 1..=4 and loads through a fixed spread so weighted and
/// load-aware policies do real comparison work rather than repeatedly meeting
/// ties. Identities are stable across calls, so a rebuild triggered by a
/// membership change is triggered by the membership and not by renaming.
pub(crate) fn candidates(count: usize) -> Vec<Candidate> {
    (0..count)
        .map(|index| {
            let weight =
                Weight::new(u32::try_from(index % 4 + 1).unwrap_or(1)).unwrap_or(Weight::ONE);
            let load = Load::try_from(index % 17).unwrap_or(0);
            Backend::from_parts(
                format!("backend-{index:04}"),
                (),
                load,
                weight,
                Status::Ready,
            )
        })
        .collect()
}

/// Builds `count` candidates with one removed from the middle.
///
/// Used to force the cached policies to rebuild. Removing from the middle
/// rather than the end keeps the change from being a truncation, which some
/// membership fingerprints would notice more cheaply than a real reshuffle.
pub(crate) fn candidates_missing_one(count: usize) -> Vec<Candidate> {
    let mut set = candidates(count);
    if set.len() > 1 {
        set.remove(set.len() / 2);
    }
    set
}

/// Wraps candidates in priority tiers, cycling across three tiers.
pub(crate) fn prioritized(count: usize) -> Vec<Prioritized<Candidate>> {
    candidates(count)
        .into_iter()
        .enumerate()
        .map(|(index, backend)| Prioritized::new(backend, u32::try_from(index % 3).unwrap_or(0)))
        .collect()
}

/// Wraps prioritized candidates across three localities.
pub(crate) fn localized(count: usize) -> Vec<Localized<Prioritized<Candidate>, &'static str>> {
    const LOCALITIES: [&str; 3] = ["us-west", "us-east", "eu-west"];
    prioritized(count)
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| Localized::new(candidate, LOCALITIES[index % LOCALITIES.len()]))
        .collect()
}

/// Request keys for the affinity policies.
///
/// A keyed policy measured against one repeated key measures a warm branch
/// predictor, not the policy. The count is coprime with every candidate count
/// above so the key and candidate cycles do not align.
pub(crate) fn keys() -> Vec<String> {
    (0..997)
        .map(|index| format!("request-{index:05}"))
        .collect()
}
