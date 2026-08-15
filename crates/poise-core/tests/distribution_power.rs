//! Distribution laws with a stated sensitivity.
//!
//! A test that asserts counts fall inside a hand-picked band is a hypothesis
//! test whose operating point nobody wrote down. The in-module tests use bands
//! about six standard deviations wide, which is the right *shape* -- a false
//! alarm is then about one in a billion, so the suite does not flake -- but at
//! their sample sizes six standard deviations is also five percent of the
//! expected count. A sampler biased by four percent passes.
//!
//! These tests keep the stringent threshold and fix the other half: the sample
//! size is derived from the deviation the test claims to detect, and the test
//! asserts that its own design is powerful enough before it looks at the data.
//! An underpowered test fails as loudly as a biased sampler, so the sensitivity
//! cannot quietly decay as the code around it changes.
//!
//! Sample sizes come out in the millions and cost tens of milliseconds, which
//! the recorded selection baselines make predictable rather than surprising.

// Counts and sample sizes stay far below 2^53, so every conversion here is
// exact. The alternative is threading integer arithmetic through a variance
// calculation, which would obscure the statistics without improving them.
#![allow(clippy::cast_precision_loss)]

use poise_core::{
    Backend, Policy, Status, Weight,
    policy::{Random, Rendezvous, WeightedRandom, WeightedRendezvous},
};

/// Per-bin rejection threshold in standard deviations.
///
/// A two-sided normal tail beyond 6.8 sigma is about 1e-11. The suite performs
/// fewer than a hundred such comparisons, so the family-wise false-alarm rate
/// stays near 1e-9: this suite should never flake in the lifetime of the
/// project, which is a precondition for a gate that blocks merges.
const THRESHOLD_SIGMA: f64 = 6.8;

/// Additional margin so a deviation at the claimed size is actually detected.
///
/// Sizing a sample so the claimed deviation lands exactly on the rejection
/// boundary buys a detection probability of about one half: the estimate is
/// centred on the boundary, so it falls short as often as it clears it. A test
/// with fifty percent power is not a test, it is a coin flip that happens to
/// agree with the truth on average.
///
/// The margin is the normal quantile for the detection rate we want, so the
/// sample must place the claimed deviation `THRESHOLD_SIGMA + POWER_SIGMA`
/// standard deviations out. At 1.645 the detection rate is ninety-five percent,
/// one-sided, which is the direction a bias moves a bin.
const POWER_SIGMA: f64 = 1.645;

/// Margin over the computed size, absorbing the spread between standard forms.
///
/// Several textbook expressions approximate a binomial power calculation and
/// they disagree by a fraction of a percent. Checked against an independent
/// exact implementation, the expression below lands between 0.15% and 0.40%
/// *under* the required count, and always under rather than over. A systematic
/// shortfall is precisely the quiet erosion the power assertion exists to
/// prevent, so the size carries a margin wider than the observed disagreement.
/// Two percent of a two-million-draw sample is milliseconds.
const SAFETY_MARGIN: f64 = 1.02;

/// Smallest sample size that detects a `relative` deviation at the target power.
///
/// The standard two-proportion form, with the null variance under the rejection
/// threshold and the alternative's variance under the power term:
///
/// ```text
/// n >= (Z * sqrt(p(1-p)) + Z_power * sqrt(q(1-q)))^2 / (q - p)^2,  q = p(1 + relative)
/// ```
///
/// Using the null variance for both terms, which is the tidier expression, is
/// what put this below the required count before it was checked. The binding
/// bin is the least likely one, so the caller passes the smallest probability.
fn required_samples(smallest_probability: f64, relative: f64) -> u64 {
    assert!(
        smallest_probability > 0.0 && relative > 0.0,
        "a sample size is only defined for a positive probability and tolerance"
    );
    let null = smallest_probability;
    let alternative = null * (1.0 + relative);
    assert!(
        alternative < 1.0,
        "a relative deviation of {relative} takes probability {null} past one"
    );

    let separation = alternative - null;
    let spread = THRESHOLD_SIGMA * (null * (1.0 - null)).sqrt()
        + POWER_SIGMA * (alternative * (1.0 - alternative)).sqrt();
    let samples = (spread * spread / (separation * separation) * SAFETY_MARGIN).ceil();

    assert!(
        samples.is_finite() && samples < 1e15,
        "required sample size {samples} is not a workable count"
    );
    // Guarded above: positive, finite, and far below `u64::MAX`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let samples = samples as u64;
    samples
}

/// Asserts observed counts match expected probabilities, and that the test could
/// have noticed if they had not.
///
/// The power assertion is the point. Without it this is the same
/// hand-picked-band test with more arithmetic: a future change that shrinks the
/// sample would silently widen the tolerance instead of failing. It is stated
/// against `required_samples` rather than restating the criterion, so the
/// sizing rule and the check on it cannot drift apart.
fn assert_distribution(label: &str, counts: &[u64], probabilities: &[f64], relative: f64) {
    let total: u64 = counts.iter().sum();
    let samples = total as f64;
    let smallest = probabilities.iter().copied().fold(f64::INFINITY, f64::min);
    let needed = required_samples(smallest, relative);

    assert!(
        total >= needed,
        "{label}: {total} samples cannot detect a {:.3}% deviation at the target \
         power, which needs {needed}",
        100.0 * relative
    );

    for (index, (count, probability)) in counts.iter().zip(probabilities).enumerate() {
        let expected = probability * samples;
        let sigma = (samples * probability * (1.0 - probability)).sqrt();
        let deviation = (*count as f64 - expected) / sigma;
        assert!(
            deviation.abs() < THRESHOLD_SIGMA,
            "{label}: bin {index} saw {count} against {expected:.1} expected, \
             {deviation:.2} sigma away, past the {THRESHOLD_SIGMA} sigma threshold"
        );
    }
}

fn weighted(index: usize, weight: u32) -> Backend<String, (), u64> {
    Backend::from_parts(
        format!("backend-{index}"),
        (),
        0,
        Weight::new(weight).expect("weight is non-zero"),
        Status::Ready,
    )
}

fn shares(weights: &[u32]) -> Vec<f64> {
    let total: f64 = weights.iter().map(|weight| f64::from(*weight)).sum();
    weights
        .iter()
        .map(|weight| f64::from(*weight) / total)
        .collect()
}

/// Seeds swept by the randomized policies.
///
/// A single seed makes a distribution test a regression test on one draw: a
/// sampler biased for every seed but this one passes forever. A fixed *set*
/// keeps the run reproducible, which the mutation gate needs, while sampling
/// several points of the seed space instead of one.
const SEEDS: [u64; 8] = [1, 7, 11, 19, 23, 101, 1_009, 65_537];

fn sample_random(weights: &[u32], samples: u64) -> Vec<u64> {
    let candidates: Vec<_> = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| weighted(index, *weight))
        .collect();
    let mut counts = vec![0_u64; weights.len()];
    let per_seed = samples / SEEDS.len() as u64 + 1;

    for seed in SEEDS {
        let mut uniform = Random::seeded(seed);
        let mut proportional = WeightedRandom::seeded(seed);
        for _ in 0..per_seed {
            let index = if weights.iter().all(|weight| *weight == weights[0]) {
                uniform.pick(&candidates, &()).unwrap().index()
            } else {
                proportional.pick(&candidates, &()).unwrap().index()
            };
            counts[index] += 1;
        }
    }

    counts
}

/// Uniform selection is uniform to within one percent.
///
/// The in-module test resolves about five percent at three candidates. This
/// resolves one, which is the difference between noticing a biased index
/// calculation and noticing only a broken one.
#[test]
#[ignore = "millions of samples: cheap in release, ~3s in debug, and the \
           mutation campaign would pay that once per mutant"]
fn random_selection_is_uniform_to_one_percent() {
    for size in [3_usize, 5] {
        let weights = vec![1_u32; size];
        let probabilities = shares(&weights);
        let samples = required_samples(1.0 / size as f64, 0.01);
        let counts = sample_random(&weights, samples);

        assert_distribution(
            &format!("uniform over {size}"),
            &counts,
            &probabilities,
            0.01,
        );
    }
}

/// Weighted selection tracks its weights to within one percent.
#[test]
#[ignore = "millions of samples: cheap in release, ~3s in debug, and the \
           mutation campaign would pay that once per mutant"]
fn weighted_selection_tracks_weights_to_one_percent() {
    for weights in [vec![1_u32, 3], vec![1, 2, 3, 4]] {
        let probabilities = shares(&weights);
        let smallest = probabilities.iter().copied().fold(f64::INFINITY, f64::min);
        let samples = required_samples(smallest, 0.01);
        let counts = sample_random(&weights, samples);

        assert_distribution(
            &format!("weights {weights:?}"),
            &counts,
            &probabilities,
            0.01,
        );
    }
}

fn sample_keyspace<P>(policy: &mut P, weights: &[u32], samples: u64) -> Vec<u64>
where
    P: Policy<Backend<String, (), u64>, u64>,
{
    let candidates: Vec<_> = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| weighted(index, *weight))
        .collect();
    let mut counts = vec![0_u64; weights.len()];

    for key in 0..samples {
        counts[policy.pick(&candidates, &key).unwrap().index()] += 1;
    }

    counts
}

/// Rendezvous gives every candidate an equal share of the keyspace.
///
/// The share is exact rather than incidental: the winner of a key is the
/// `argmax` of independent per-candidate draws, so each candidate wins with
/// probability `1/n`. Deviation past the threshold indicates a hash that fails
/// to spread keys, which is the failure this policy family cannot tolerate.
#[test]
#[ignore = "millions of samples: cheap in release, ~3s in debug, and the \
           mutation campaign would pay that once per mutant"]
fn rendezvous_spreads_the_keyspace_evenly() {
    let weights = vec![1_u32; 4];
    let probabilities = shares(&weights);
    let relative = 0.02;
    let samples = required_samples(0.25, relative);
    let mut policy = Rendezvous::new();
    let counts = sample_keyspace(&mut policy, &weights, samples);

    assert_distribution("rendezvous keyspace", &counts, &probabilities, relative);
}

/// Weighted rendezvous gives each candidate its weighted share of the keyspace.
///
/// Exponential score transformation makes the win probability exactly
/// proportional to weight, so this is the same multinomial law as the
/// randomized policies with the key playing the part of the draw.
#[test]
#[ignore = "millions of samples: cheap in release, ~3s in debug, and the \
           mutation campaign would pay that once per mutant"]
fn weighted_rendezvous_spreads_the_keyspace_by_weight() {
    let weights = vec![1_u32, 3];
    let probabilities = shares(&weights);
    let relative = 0.02;
    let smallest = probabilities.iter().copied().fold(f64::INFINITY, f64::min);
    let samples = required_samples(smallest, relative);
    let mut policy = WeightedRendezvous::new();
    let counts = sample_keyspace(&mut policy, &weights, samples);

    assert_distribution(
        "weighted rendezvous keyspace",
        &counts,
        &probabilities,
        relative,
    );
}

/// The power assertion rejects a sample too small for the claimed sensitivity.
///
/// Guards the guard. If `assert_distribution` stopped checking power it would
/// still pass every test above, because those samples are generously sized --
/// the failure would only appear later, as a test that had quietly stopped
/// detecting anything.
#[test]
#[should_panic(expected = "cannot detect")]
fn an_undersized_sample_is_rejected_rather_than_tolerated() {
    let counts = [500_u64, 500];
    assert_distribution("deliberately tiny", &counts, &[0.5, 0.5], 0.01);
}

/// A sampler biased past the claimed sensitivity is caught.
///
/// The other half of guarding the guard: confirms the threshold rejects a
/// deviation just past what the test claims to resolve, rather than being wide
/// enough to admit it.
#[test]
#[should_panic(expected = "sigma away")]
fn a_biased_sample_is_rejected() {
    // Twice the minimum, so the power assertion clears comfortably and the
    // panic this test expects is the deviation one. At exactly the minimum the
    // two assertions sit on the same boundary and rounding decides which fires.
    let samples = required_samples(0.5, 0.01) * 2;
    let half = samples / 2;
    // Two percent high against a one percent claim.
    let skew = half / 50;
    let counts = [half + skew, half - skew];
    assert_distribution("deliberately biased", &counts, &[0.5, 0.5], 0.01);
}

/// Sizing stays at or above an independently computed requirement.
///
/// The reference counts come from an exact one-proportion power calculation
/// performed outside this repository, at the same threshold and power target.
/// They are recorded rather than recomputed because the point is to disagree
/// with this file's arithmetic if it drifts, which a second copy of the same
/// expression could not do.
///
/// The expression here was 0.15% to 0.40% *below* these before the margin was
/// added, always short and never over, which is how a stated sensitivity decays
/// into a slogan.
#[test]
fn sizing_meets_an_independently_computed_requirement() {
    // (smallest probability, relative deviation, exact required samples)
    let references = [
        (1.0 / 3.0, 0.01, 1_429_852_u64),
        (0.25, 0.01, 2_146_569),
        (0.2, 0.01, 2_863_287),
        (0.1, 0.01, 6_446_873),
        (0.25, 0.02, 538_403),
    ];

    for (probability, relative, exact) in references {
        let computed = required_samples(probability, relative);
        assert!(
            computed >= exact,
            "p={probability}, r={relative}: sized {computed} against a requirement of {exact}"
        );
        // Generous is fine, wasteful is not: a sample far above the requirement
        // would mean the margin had swallowed a real error in the expression.
        assert!(
            computed <= exact * 11 / 10,
            "p={probability}, r={relative}: sized {computed}, more than a tenth \
             above the {exact} required"
        );
    }
}
