use std::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroUsize},
    time::{Duration, Instant},
};

#[cfg(loom)]
use loom::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex, MutexGuard};

/// Bounds on a [`ProbePool`]'s size, entry reuse, and entry age.
///
/// There is deliberately no `Default`. A pool's constants determine how stale
/// and how widely shared an observation may be before it steers traffic, and
/// those are deployment properties rather than library ones.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProbePoolConfig {
    capacity: NonZeroUsize,
    max_uses: NonZeroU32,
    max_age: Duration,
}

impl ProbePoolConfig {
    /// Creates a pool configuration.
    ///
    /// `capacity` bounds retained observations. `max_uses` bounds how many
    /// decisions one observation may inform before it is discarded. `max_age`
    /// bounds how long an observation remains usable.
    ///
    /// # Errors
    ///
    /// Returns [`ProbePoolConfigError::ZeroMaxAge`] when `max_age` is zero,
    /// which would expire every observation before it could be read.
    pub const fn new(
        capacity: NonZeroUsize,
        max_uses: NonZeroU32,
        max_age: Duration,
    ) -> Result<Self, ProbePoolConfigError> {
        if max_age.is_zero() {
            return Err(ProbePoolConfigError::ZeroMaxAge);
        }
        Ok(Self {
            capacity,
            max_uses,
            max_age,
        })
    }

    /// Returns the maximum number of retained observations.
    #[must_use]
    pub const fn capacity(self) -> NonZeroUsize {
        self.capacity
    }

    /// Returns the maximum number of decisions one observation may inform.
    #[must_use]
    pub const fn max_uses(self) -> NonZeroU32 {
        self.max_uses
    }

    /// Returns the maximum age at which an observation remains usable.
    #[must_use]
    pub const fn max_age(self) -> Duration {
        self.max_age
    }
}

/// Invalid probe-pool configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ProbePoolConfigError {
    /// A zero maximum age expires every observation before it can be read.
    ZeroMaxAge,
}

impl fmt::Display for ProbePoolConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaxAge => f.write_str("probe pool maximum age must be non-zero"),
        }
    }
}

impl Error for ProbePoolConfigError {}

/// One out-of-band observation of a replica.
///
/// Requests-in-flight is the replica's own reported queue depth, not a count
/// maintained by the caller. Latency is the observed service time for the probe
/// itself.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProbeReading {
    requests_in_flight: u64,
    latency: Duration,
}

impl ProbeReading {
    /// Creates a reading from a reported queue depth and observed latency.
    #[must_use]
    pub const fn new(requests_in_flight: u64, latency: Duration) -> Self {
        Self {
            requests_in_flight,
            latency,
        }
    }

    /// Returns the replica's reported requests-in-flight.
    #[must_use]
    pub const fn requests_in_flight(self) -> u64 {
        self.requests_in_flight
    }

    /// Returns the observed probe latency.
    #[must_use]
    pub const fn latency(self) -> Duration {
        self.latency
    }
}

/// A retained observation together with its remaining reuse budget.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProbeEntry<Id> {
    id: Id,
    reading: ProbeReading,
    recorded_at: Instant,
    remaining_uses: u32,
}

impl<Id> ProbeEntry<Id> {
    /// Returns the observed replica's identity.
    pub const fn id(&self) -> &Id {
        &self.id
    }

    /// Returns the observation.
    #[must_use]
    pub const fn reading(&self) -> ProbeReading {
        self.reading
    }

    /// Returns the replica's reported requests-in-flight.
    #[must_use]
    pub const fn requests_in_flight(&self) -> u64 {
        self.reading.requests_in_flight()
    }

    /// Returns the observed probe latency.
    #[must_use]
    pub const fn latency(&self) -> Duration {
        self.reading.latency()
    }

    /// Returns how many further decisions this observation may inform.
    ///
    /// A returned entry reports the budget remaining *after* the decision that
    /// produced it, so zero means the entry was retired by that decision.
    #[must_use]
    pub const fn remaining_uses(&self) -> u32 {
        self.remaining_uses
    }

    /// Returns the observation's age at `now`, saturating at zero.
    #[must_use]
    pub fn age_at(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.recorded_at)
    }
}

/// Why a pool could not produce a decision.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ProbeDecisionError {
    /// No unexpired observation was available.
    ///
    /// This is the ordinary cold-start and idle-pool outcome, not a failure.
    /// A caller is expected to fall back to a policy that needs no probe data.
    NoProbes,
    /// Observations were available, but the selector chose none of them.
    NoSelection,
    /// The selector returned an index outside the slice it was given.
    IndexOutOfBounds,
}

impl fmt::Display for ProbeDecisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProbes => f.write_str("no unexpired probe observation is available"),
            Self::NoSelection => f.write_str("the selector rejected every available observation"),
            Self::IndexOutOfBounds => {
                f.write_str("the selector returned an out-of-bounds observation index")
            }
        }
    }
}

impl Error for ProbeDecisionError {}

struct PoolState<Id> {
    entries: Vec<ProbeEntry<Id>>,
}

/// A bounded, self-expiring pool of replica observations.
///
/// The pool holds what recent out-of-band probes reported so a policy can rank
/// replicas by latency rather than by a load counter the caller maintains. It
/// stores observations and enforces their lifetime; it does not rank them, and
/// it has no opinion about which replica should be selected.
///
/// Clones share all state, so a prober and the selectors it feeds hold the same
/// pool.
///
/// # Retention contract
///
/// - **Bounded.** At most `capacity` observations are retained. Recording into
///   a full pool evicts the oldest.
/// - **Consumed on use.** An observation informs at most `max_uses` decisions
///   and is then discarded. This is a correctness property rather than an
///   optimization: an observation reporting an idle replica that every
///   concurrent selector could read would steer all of them onto that replica
///   at once. [`ProbePool::decide_at`] charges the use under the same lock that
///   exposes the entry, so concurrent selectors cannot overspend one
///   observation.
/// - **Aged out.** An observation at or beyond `max_age` never informs a
///   decision, whatever its remaining reuse budget.
///
/// The pool does not evaluate eligibility. An entry may name a replica that has
/// since begun draining or left the membership generation, so a caller must
/// filter against its own candidate set inside the selector.
///
/// # Time
///
/// Every time-dependent operation has an `_at` form that takes the reading
/// instant, following [`crate::PeakEwma`] and the active-health prober. Tests
/// and simulations drive a synthetic clock through those; the convenience forms
/// read [`Instant::now`].
///
/// # Example
///
/// ```
/// use std::{num::{NonZeroU32, NonZeroUsize}, time::Duration};
/// use poise_core::{ProbePool, ProbePoolConfig, ProbeReading};
///
/// let config = ProbePoolConfig::new(
///     NonZeroUsize::new(16).unwrap(),
///     NonZeroU32::new(1).unwrap(),
///     Duration::from_secs(1),
/// )?;
/// let pool = ProbePool::new(config);
///
/// pool.record("west", ProbeReading::new(7, Duration::from_millis(20)));
/// pool.record("east", ProbeReading::new(2, Duration::from_millis(35)));
///
/// let decision = pool.decide(|entries| {
///     entries
///         .iter()
///         .enumerate()
///         .min_by_key(|(_, entry)| entry.requests_in_flight())
///         .map(|(index, _)| index)
/// })?;
///
/// assert_eq!(decision.id(), &"east");
/// assert_eq!(decision.remaining_uses(), 0);
/// assert_eq!(pool.len_at(std::time::Instant::now()), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct ProbePool<Id> {
    config: ProbePoolConfig,
    state: Arc<Mutex<PoolState<Id>>>,
}

impl<Id> Clone for ProbePool<Id> {
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            state: Arc::clone(&self.state),
        }
    }
}

impl<Id> ProbePool<Id> {
    /// Creates an empty pool.
    #[must_use]
    pub fn new(config: ProbePoolConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(PoolState {
                entries: Vec::new(),
            })),
        }
    }

    /// Returns the configuration.
    #[must_use]
    pub const fn config(&self) -> ProbePoolConfig {
        self.config
    }

    /// Records an observation, evicting the oldest entry when full.
    pub fn record(&self, id: Id, reading: ProbeReading) {
        self.record_at(id, reading, Instant::now());
    }

    /// Records an observation taken at `now`.
    ///
    /// A full pool evicts its oldest retained observation, which is the one
    /// closest to expiry and therefore the least informative.
    pub fn record_at(&self, id: Id, reading: ProbeReading, now: Instant) {
        let mut state = self.lock();
        self.expire(&mut state, now);

        if state.entries.len() >= self.config.capacity.get() {
            if let Some(oldest) = Self::oldest_index(&state) {
                state.entries.remove(oldest);
            }
        }

        state.entries.push(ProbeEntry {
            id,
            reading,
            recorded_at: now,
            remaining_uses: self.config.max_uses.get(),
        });
    }

    /// Returns the number of unexpired observations.
    #[must_use]
    pub fn len_at(&self, now: Instant) -> usize {
        let mut state = self.lock();
        self.expire(&mut state, now);
        state.entries.len()
    }

    /// Returns whether no unexpired observation is retained.
    #[must_use]
    pub fn is_empty_at(&self, now: Instant) -> bool {
        self.len_at(now) == 0
    }

    /// Discards every retained observation.
    pub fn clear(&self) {
        self.lock().entries.clear();
    }

    /// Selects one observation and charges its reuse budget.
    ///
    /// See [`ProbePool::decide_at`].
    ///
    /// # Errors
    ///
    /// Returns [`ProbeDecisionError`] when no observation is available, the
    /// selector chose none, or the selector returned an invalid index.
    pub fn decide<F>(&self, choose: F) -> Result<ProbeEntry<Id>, ProbeDecisionError>
    where
        Id: Clone,
        F: FnOnce(&[ProbeEntry<Id>]) -> Option<usize>,
    {
        self.decide_at(Instant::now(), choose)
    }

    /// Selects one observation as of `now` and charges its reuse budget.
    ///
    /// Expired observations are dropped first, then `choose` receives the
    /// remaining slice and returns the index it selects. The chosen entry's
    /// reuse budget is decremented, and the entry is discarded once exhausted.
    ///
    /// Expiry, selection, and charging happen under one lock, which is what
    /// makes the reuse bound hold across concurrent selectors. `choose` must
    /// therefore be a pure ranking function: it runs while the pool is locked,
    /// so it must not call back into this pool.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeDecisionError::NoProbes`] when nothing unexpired is
    /// retained, [`ProbeDecisionError::NoSelection`] when `choose` returns
    /// `None`, and [`ProbeDecisionError::IndexOutOfBounds`] when `choose`
    /// returns an index outside the slice it was given. The pool is left
    /// unchanged except for expiry in every error case.
    pub fn decide_at<F>(
        &self,
        now: Instant,
        choose: F,
    ) -> Result<ProbeEntry<Id>, ProbeDecisionError>
    where
        Id: Clone,
        F: FnOnce(&[ProbeEntry<Id>]) -> Option<usize>,
    {
        let mut state = self.lock();
        self.expire(&mut state, now);
        if state.entries.is_empty() {
            return Err(ProbeDecisionError::NoProbes);
        }

        let index = choose(&state.entries).ok_or(ProbeDecisionError::NoSelection)?;
        if index >= state.entries.len() {
            return Err(ProbeDecisionError::IndexOutOfBounds);
        }

        let entry = &mut state.entries[index];
        entry.remaining_uses = entry.remaining_uses.saturating_sub(1);
        let decided = entry.clone();
        if decided.remaining_uses == 0 {
            state.entries.remove(index);
        }
        Ok(decided)
    }

    fn expire(&self, state: &mut PoolState<Id>, now: Instant) {
        let max_age = self.config.max_age;
        state.entries.retain(|entry| entry.age_at(now) < max_age);
    }

    fn oldest_index(state: &PoolState<Id>) -> Option<usize> {
        state
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.recorded_at)
            .map(|(index, _)| index)
    }

    fn lock(&self) -> MutexGuard<'_, PoolState<Id>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<Id> fmt::Debug for ProbePool<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProbePool")
            .field("capacity", &self.config.capacity())
            .field("max_uses", &self.config.max_uses())
            .field("max_age", &self.config.max_age())
            .finish_non_exhaustive()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn config(capacity: usize, max_uses: u32, max_age: Duration) -> ProbePoolConfig {
        ProbePoolConfig::new(
            NonZeroUsize::new(capacity).unwrap(),
            NonZeroU32::new(max_uses).unwrap(),
            max_age,
        )
        .unwrap()
    }

    fn reading(requests_in_flight: u64, latency_millis: u64) -> ProbeReading {
        ProbeReading::new(requests_in_flight, Duration::from_millis(latency_millis))
    }

    fn lowest_latency(entries: &[ProbeEntry<&'static str>]) -> Option<usize> {
        entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.latency())
            .map(|(index, _)| index)
    }

    #[test]
    fn configuration_rejects_a_zero_maximum_age() {
        assert_eq!(
            ProbePoolConfig::new(NonZeroUsize::MIN, NonZeroU32::MIN, Duration::ZERO),
            Err(ProbePoolConfigError::ZeroMaxAge)
        );
        assert_eq!(config(4, 2, Duration::from_secs(1)).capacity().get(), 4);
    }

    #[test]
    fn recording_never_exceeds_capacity_and_evicts_the_oldest() {
        let pool = ProbePool::new(config(2, 1, Duration::from_secs(60)));
        let base = Instant::now();

        pool.record_at("a", reading(1, 10), base);
        pool.record_at("b", reading(2, 20), base + Duration::from_millis(1));
        pool.record_at("c", reading(3, 30), base + Duration::from_millis(2));

        let now = base + Duration::from_millis(3);
        assert_eq!(pool.len_at(now), 2);

        let survivors = [
            pool.decide_at(now, |_| Some(0)).unwrap(),
            pool.decide_at(now, |_| Some(0)).unwrap(),
        ];
        let mut ids: Vec<_> = survivors.iter().map(|entry| *entry.id()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["b", "c"]);
    }

    #[test]
    fn an_entry_informs_exactly_its_configured_number_of_decisions() {
        let pool = ProbePool::new(config(4, 3, Duration::from_secs(60)));
        let now = Instant::now();
        pool.record_at("a", reading(1, 10), now);

        for expected_remaining in [2, 1, 0] {
            let decided = pool.decide_at(now, |_| Some(0)).unwrap();
            assert_eq!(decided.remaining_uses(), expected_remaining);
        }

        assert_eq!(
            pool.decide_at(now, |_| Some(0)),
            Err(ProbeDecisionError::NoProbes)
        );
        assert!(pool.is_empty_at(now));
    }

    #[test]
    fn an_expired_entry_never_informs_a_decision() {
        let max_age = Duration::from_millis(100);
        let pool = ProbePool::new(config(4, 8, max_age));
        let base = Instant::now();
        pool.record_at("a", reading(1, 10), base);

        assert_eq!(pool.len_at(base + Duration::from_millis(99)), 1);
        assert_eq!(pool.len_at(base + max_age), 0);
        assert_eq!(
            pool.decide_at(base + max_age, |_| Some(0)),
            Err(ProbeDecisionError::NoProbes)
        );
    }

    #[test]
    fn expiry_precedes_selection_so_a_selector_only_sees_live_entries() {
        let max_age = Duration::from_millis(100);
        let pool = ProbePool::new(config(4, 1, max_age));
        let base = Instant::now();
        pool.record_at("stale", reading(0, 1), base);
        pool.record_at("fresh", reading(9, 50), base + Duration::from_millis(80));

        let decided = pool
            .decide_at(base + Duration::from_millis(120), lowest_latency)
            .unwrap();

        assert_eq!(decided.id(), &"fresh");
    }

    #[test]
    fn selector_outcomes_stay_distinguishable() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        let now = Instant::now();

        assert_eq!(
            pool.decide_at(now, |_| Some(0)),
            Err(ProbeDecisionError::NoProbes)
        );

        pool.record_at("a", reading(1, 10), now);
        assert_eq!(
            pool.decide_at(now, |_| None),
            Err(ProbeDecisionError::NoSelection)
        );
        assert_eq!(
            pool.decide_at(now, |entries| Some(entries.len())),
            Err(ProbeDecisionError::IndexOutOfBounds)
        );
        assert_eq!(pool.len_at(now), 1, "a rejected decision spends no budget");
        assert_eq!(pool.decide_at(now, |_| Some(0)).unwrap().id(), &"a");
    }

    #[test]
    fn clones_share_one_pool() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        let prober = pool.clone();
        let now = Instant::now();

        prober.record_at("a", reading(1, 10), now);

        assert_eq!(pool.len_at(now), 1);
        assert_eq!(pool.decide_at(now, |_| Some(0)).unwrap().id(), &"a");
        assert!(prober.is_empty_at(now));
    }

    #[test]
    fn clearing_discards_every_observation() {
        let pool = ProbePool::new(config(4, 2, Duration::from_secs(60)));
        let now = Instant::now();
        pool.record_at("a", reading(1, 10), now);
        pool.record_at("b", reading(2, 20), now);
        assert!(!pool.is_empty_at(now), "a populated pool reports non-empty");

        pool.clear();

        assert!(pool.is_empty_at(now));
    }

    #[test]
    fn an_entry_reports_its_age() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        let base = Instant::now();
        let recorded_at = base + Duration::from_millis(5);
        pool.record_at("a", reading(1, 10), recorded_at);

        let decided = pool
            .decide_at(recorded_at + Duration::from_millis(25), |_| Some(0))
            .unwrap();

        assert_eq!(
            decided.age_at(recorded_at + Duration::from_millis(25)),
            Duration::from_millis(25)
        );
        assert_eq!(
            decided.age_at(base),
            Duration::ZERO,
            "an instant before the observation saturates rather than underflowing"
        );
    }

    #[test]
    fn eviction_uses_observation_time_rather_than_insertion_order() {
        let pool = ProbePool::new(config(2, 1, Duration::from_secs(60)));
        let base = Instant::now();

        // Probes complete out of order, so a later recording can carry an
        // earlier observation instant. Eviction must follow the instant.
        pool.record_at("newest", reading(0, 1), base + Duration::from_millis(20));
        pool.record_at("oldest", reading(0, 2), base + Duration::from_millis(5));
        pool.record_at("newer", reading(0, 3), base + Duration::from_millis(30));

        let now = base + Duration::from_millis(40);
        let mut retained = vec![
            *pool.decide_at(now, |_| Some(0)).unwrap().id(),
            *pool.decide_at(now, |_| Some(0)).unwrap().id(),
        ];
        retained.sort_unstable();

        assert_eq!(retained, ["newer", "newest"]);
    }

    #[test]
    fn a_selector_ranks_the_whole_live_slice() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        let now = Instant::now();
        pool.record_at("slow", reading(0, 90), now);
        pool.record_at("fast", reading(0, 5), now);
        pool.record_at("medium", reading(0, 40), now);

        let decided = pool.decide_at(now, lowest_latency).unwrap();

        assert_eq!(
            decided.id(),
            &"fast",
            "the winner is not at the head, so the slice must be ranked"
        );
    }

    #[test]
    fn readings_are_reported_unchanged() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        let now = Instant::now();
        let observed = reading(42, 7);
        pool.record_at("a", observed, now);

        let decided = pool.decide_at(now, |_| Some(0)).unwrap();

        assert_eq!(decided.reading(), observed);
        assert_eq!(decided.requests_in_flight(), 42);
        assert_eq!(decided.latency(), Duration::from_millis(7));
    }

    #[test]
    fn debug_reports_bounds_without_entry_contents() {
        let pool = ProbePool::new(config(4, 1, Duration::from_secs(60)));
        pool.record_at("secret", reading(1, 10), Instant::now());

        let rendered = format!("{pool:?}");

        assert!(rendered.contains("capacity"));
        assert!(!rendered.contains("secret"));
    }
}
