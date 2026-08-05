use std::{error::Error, fmt, num::NonZeroUsize};

use crate::OutcomeStats;

/// Group-relative success-rate outlier configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlierConfig {
    minimum_hosts: NonZeroUsize,
    minimum_samples: NonZeroUsize,
    standard_deviation_factor: f64,
    max_ejection_percent: u8,
    minimum_healthy: usize,
}

impl OutlierConfig {
    /// Creates an outlier configuration.
    ///
    /// A backend is a candidate when its success rate is below
    /// `mean - standard_deviation_factor * standard_deviation`.
    ///
    /// # Errors
    ///
    /// Returns [`OutlierConfigError`] for a negative/non-finite deviation factor
    /// or an ejection percentage above 100.
    pub fn new(
        minimum_hosts: NonZeroUsize,
        minimum_samples: NonZeroUsize,
        standard_deviation_factor: f64,
        max_ejection_percent: u8,
        minimum_healthy: usize,
    ) -> Result<Self, OutlierConfigError> {
        if !standard_deviation_factor.is_finite() || standard_deviation_factor < 0.0 {
            return Err(OutlierConfigError::InvalidStandardDeviationFactor);
        }
        if max_ejection_percent > 100 {
            return Err(OutlierConfigError::InvalidMaxEjectionPercent);
        }
        Ok(Self {
            minimum_hosts,
            minimum_samples,
            standard_deviation_factor,
            max_ejection_percent,
            minimum_healthy,
        })
    }

    /// Returns the hosts required to establish a group baseline.
    #[must_use]
    pub const fn minimum_hosts(self) -> NonZeroUsize {
        self.minimum_hosts
    }

    /// Returns the observations required from each considered host.
    #[must_use]
    pub const fn minimum_samples(self) -> NonZeroUsize {
        self.minimum_samples
    }

    /// Returns the standard-deviation multiplier.
    #[must_use]
    pub const fn standard_deviation_factor(self) -> f64 {
        self.standard_deviation_factor
    }

    /// Returns the maximum percentage of the full group that may be ejected.
    #[must_use]
    pub const fn max_ejection_percent(self) -> u8 {
        self.max_ejection_percent
    }

    /// Returns the minimum group size preserved after ejection.
    #[must_use]
    pub const fn minimum_healthy(self) -> usize {
        self.minimum_healthy
    }
}

impl Default for OutlierConfig {
    fn default() -> Self {
        Self::new(
            NonZeroUsize::new(5).expect("five is non-zero"),
            NonZeroUsize::new(50).expect("fifty is non-zero"),
            1.9,
            50,
            1,
        )
        .expect("the default outlier configuration is valid")
    }
}

/// Invalid group-relative outlier configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum OutlierConfigError {
    /// The deviation factor must be finite and non-negative.
    InvalidStandardDeviationFactor,
    /// An ejection percentage cannot exceed 100.
    InvalidMaxEjectionPercent,
}

impl fmt::Display for OutlierConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStandardDeviationFactor => {
                f.write_str("outlier standard-deviation factor must be finite and non-negative")
            }
            Self::InvalidMaxEjectionPercent => {
                f.write_str("maximum outlier ejection percentage cannot exceed 100")
            }
        }
    }
}

impl Error for OutlierConfigError {}

/// Pure group-relative success-rate outlier analysis.
#[derive(Clone, Copy, Debug, Default)]
pub struct OutlierDetector {
    config: OutlierConfig,
}

impl OutlierDetector {
    /// Creates a detector from an explicit configuration.
    #[must_use]
    pub const fn new(config: OutlierConfig) -> Self {
        Self { config }
    }

    /// Evaluates backend statistics in slice order.
    ///
    /// Returned indices address the supplied `stats` slice and are ordered from
    /// worst success rate to best. Ties retain slice order. The detector does
    /// not mutate health state.
    #[must_use]
    pub fn evaluate(&self, stats: &[OutcomeStats]) -> OutlierReport {
        let minimum_samples = self.config.minimum_samples.get();
        let mut considered = Vec::new();
        for (index, stats) in stats.iter().copied().enumerate() {
            if stats.samples() >= minimum_samples {
                if let Some(rate) = stats.success_rate() {
                    considered.push((index, rate));
                }
            }
        }

        if considered.len() < self.config.minimum_hosts.get() {
            return OutlierReport {
                considered_hosts: considered.len(),
                mean_success_rate: None,
                standard_deviation: None,
                ejection_threshold: None,
                ejected: Vec::new(),
            };
        }

        #[allow(clippy::cast_precision_loss)]
        let count = considered.len() as f64;
        let mean = considered.iter().map(|(_, rate)| rate).sum::<f64>() / count;
        let variance = considered
            .iter()
            .map(|(_, rate)| {
                let distance = rate - mean;
                distance * distance
            })
            .sum::<f64>()
            / count;
        let standard_deviation = variance.sqrt();
        let threshold =
            (mean - self.config.standard_deviation_factor * standard_deviation).max(0.0);

        let mut ejected: Vec<_> = considered
            .iter()
            .copied()
            .filter(|(_, rate)| *rate < threshold)
            .collect();
        ejected.sort_by(|(left_index, left_rate), (right_index, right_rate)| {
            left_rate
                .total_cmp(right_rate)
                .then_with(|| left_index.cmp(right_index))
        });

        let percent = usize::from(self.config.max_ejection_percent);
        let percent_cap = (stats.len() / 100) * percent + ((stats.len() % 100) * percent) / 100;
        let healthy_cap = stats.len().saturating_sub(self.config.minimum_healthy);
        ejected.truncate(percent_cap.min(healthy_cap));

        OutlierReport {
            considered_hosts: considered.len(),
            mean_success_rate: Some(mean),
            standard_deviation: Some(standard_deviation),
            ejection_threshold: Some(threshold),
            ejected: ejected.into_iter().map(|(index, _)| index).collect(),
        }
    }

    /// Returns the immutable configuration.
    #[must_use]
    pub const fn config(self) -> OutlierConfig {
        self.config
    }
}

/// The result of one outlier evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlierReport {
    considered_hosts: usize,
    mean_success_rate: Option<f64>,
    standard_deviation: Option<f64>,
    ejection_threshold: Option<f64>,
    ejected: Vec<usize>,
}

impl OutlierReport {
    /// Returns hosts with enough observations to establish the baseline.
    #[must_use]
    pub const fn considered_hosts(&self) -> usize {
        self.considered_hosts
    }

    /// Returns the group mean, or `None` when too few hosts were considered.
    #[must_use]
    pub const fn mean_success_rate(&self) -> Option<f64> {
        self.mean_success_rate
    }

    /// Returns the population standard deviation when a baseline exists.
    #[must_use]
    pub const fn standard_deviation(&self) -> Option<f64> {
        self.standard_deviation
    }

    /// Returns the exclusive ejection threshold when a baseline exists.
    #[must_use]
    pub const fn ejection_threshold(&self) -> Option<f64> {
        self.ejection_threshold
    }

    /// Returns selected indices from worst success rate to best.
    #[must_use]
    pub fn ejected(&self) -> &[usize] {
        &self.ejected
    }

    /// Returns whether a candidate index was selected for ejection.
    #[must_use]
    pub fn is_ejected(&self, index: usize) -> bool {
        self.ejected.contains(&index)
    }
}

#[cfg(test)]
mod tests {
    use poise_core::Outcome;

    use crate::OutcomeWindow;

    use super::*;

    fn config(
        minimum_hosts: usize,
        minimum_samples: usize,
        standard_deviation_factor: f64,
        max_ejection_percent: u8,
        minimum_healthy: usize,
    ) -> OutlierConfig {
        OutlierConfig::new(
            NonZeroUsize::new(minimum_hosts).unwrap(),
            NonZeroUsize::new(minimum_samples).unwrap(),
            standard_deviation_factor,
            max_ejection_percent,
            minimum_healthy,
        )
        .unwrap()
    }

    fn stats(successes: usize, failures: usize) -> OutcomeStats {
        let samples = successes + failures;
        let window = OutcomeWindow::new(NonZeroUsize::new(samples.max(1)).unwrap());
        for _ in 0..successes {
            window.record(Outcome::Success);
        }
        for _ in 0..failures {
            window.record(Outcome::Failure);
        }
        window.stats()
    }

    #[test]
    fn obvious_group_relative_outlier_is_selected() {
        let detector = OutlierDetector::new(config(5, 10, 1.0, 100, 1));
        let observations = [
            stats(10, 0),
            stats(10, 0),
            stats(10, 0),
            stats(10, 0),
            stats(0, 10),
        ];

        let report = detector.evaluate(&observations);
        assert_eq!(report.considered_hosts(), 5);
        assert_eq!(report.ejected(), &[4]);
        assert!(report.is_ejected(4));
        assert!(report.mean_success_rate().is_some());
        assert!(report.standard_deviation().is_some());
        assert!(report.ejection_threshold().is_some());
    }

    #[test]
    fn baseline_requires_enough_sampled_hosts() {
        let detector = OutlierDetector::new(config(3, 10, 1.0, 100, 0));
        let observations = [stats(10, 0), stats(9, 1), stats(1, 0)];

        let report = detector.evaluate(&observations);
        assert_eq!(report.considered_hosts(), 2);
        assert_eq!(report.mean_success_rate(), None);
        assert_eq!(report.standard_deviation(), None);
        assert_eq!(report.ejection_threshold(), None);
        assert!(report.ejected().is_empty());
    }

    #[test]
    fn percentage_cap_keeps_only_the_worst_hosts() {
        let detector = OutlierDetector::new(config(5, 10, 0.0, 40, 0));
        let observations = [
            stats(10, 0),
            stats(9, 1),
            stats(8, 2),
            stats(7, 3),
            stats(6, 4),
        ];

        assert_eq!(detector.evaluate(&observations).ejected(), &[4, 3]);
    }

    #[test]
    fn minimum_healthy_cap_can_be_stricter_than_percentage() {
        let detector = OutlierDetector::new(config(5, 10, 0.0, 100, 4));
        let observations = [
            stats(10, 0),
            stats(9, 1),
            stats(8, 2),
            stats(7, 3),
            stats(6, 4),
        ];

        assert_eq!(detector.evaluate(&observations).ejected(), &[4]);
    }

    #[test]
    fn equal_outliers_retain_slice_order() {
        let detector = OutlierDetector::new(config(4, 10, 0.0, 100, 0));
        let observations = [stats(10, 0), stats(10, 0), stats(0, 10), stats(0, 10)];

        assert_eq!(detector.evaluate(&observations).ejected(), &[2, 3]);
    }

    #[test]
    fn uniform_groups_have_no_outliers() {
        let detector = OutlierDetector::new(config(3, 10, 0.0, 100, 0));
        let observations = [stats(9, 1), stats(9, 1), stats(9, 1)];

        let report = detector.evaluate(&observations);
        assert_eq!(report.standard_deviation(), Some(0.0));
        assert!(report.ejected().is_empty());
    }

    #[test]
    fn invalid_configuration_is_rejected() {
        let hosts = NonZeroUsize::MIN;
        let samples = NonZeroUsize::MIN;
        assert_eq!(
            OutlierConfig::new(hosts, samples, -1.0, 100, 0),
            Err(OutlierConfigError::InvalidStandardDeviationFactor)
        );
        assert_eq!(
            OutlierConfig::new(hosts, samples, f64::NAN, 100, 0),
            Err(OutlierConfigError::InvalidStandardDeviationFactor)
        );
        assert_eq!(
            OutlierConfig::new(hosts, samples, 1.0, 101, 0),
            Err(OutlierConfigError::InvalidMaxEjectionPercent)
        );
    }
}
