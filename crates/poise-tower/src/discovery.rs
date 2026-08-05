use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    hash::Hash,
    task::{Context, Poll},
};

use poise_core::{Candidate, InFlight, Policy};
use poise_discovery::{Discovered, Membership, Revision, Snapshot, SnapshotReader};
use tower::Service;

use crate::{
    Balance, BalanceError, Endpoint, IgnoreReadinessErrors, LoadTracker, NoContext,
    ObserveReadinessError, RequestContext, ResponseFuture,
};

/// Builds the service and load tracker for a discovered backend generation.
///
/// The factory runs once for a new key and again when the same key points to a
/// different backend allocation. Membership-only changes retain the existing
/// service, load, and readiness state.
pub trait EndpointFactory<Key, Backend> {
    /// Tower service built for the backend.
    type Service;
    /// Dispatch load exposed to the selection policy.
    type Load: LoadTracker;
    /// Factory failure.
    type Error;

    /// Builds one service generation and its load tracker.
    ///
    /// # Errors
    ///
    /// Returns the factory's error when the endpoint cannot be constructed.
    fn build(
        &mut self,
        key: &Key,
        backend: &Backend,
    ) -> Result<(Self::Service, Self::Load), Self::Error>;
}

impl<Key, Backend, Service, Load, FactoryError, Factory> EndpointFactory<Key, Backend> for Factory
where
    Load: LoadTracker,
    Factory: FnMut(&Key, &Backend) -> Result<(Service, Load), FactoryError>,
{
    type Service = Service;
    type Load = Load;
    type Error = FactoryError;

    fn build(
        &mut self,
        key: &Key,
        backend: &Backend,
    ) -> Result<(Self::Service, Self::Load), Self::Error> {
        self(key, backend)
    }
}

/// Adapts a service-only factory to use a fresh [`InFlight`] tracker.
#[derive(Clone, Copy, Debug, Default)]
pub struct InFlightFactory<Factory> {
    inner: Factory,
}

impl<Factory> InFlightFactory<Factory> {
    /// Wraps a service factory with default in-flight tracking.
    pub const fn new(factory: Factory) -> Self {
        Self { inner: factory }
    }

    /// Returns the wrapped service factory.
    #[must_use]
    pub const fn inner(&self) -> &Factory {
        &self.inner
    }

    /// Returns mutable access to the wrapped service factory.
    pub const fn inner_mut(&mut self) -> &mut Factory {
        &mut self.inner
    }

    /// Unwraps the service factory.
    pub fn into_inner(self) -> Factory {
        self.inner
    }
}

impl<Key, Backend, Service, FactoryError, Factory> EndpointFactory<Key, Backend>
    for InFlightFactory<Factory>
where
    Factory: FnMut(&Key, &Backend) -> Result<Service, FactoryError>,
{
    type Service = Service;
    type Load = InFlight;
    type Error = FactoryError;

    fn build(
        &mut self,
        key: &Key,
        backend: &Backend,
    ) -> Result<(Self::Service, Self::Load), Self::Error> {
        (self.inner)(key, backend).map(|service| (service, InFlight::new()))
    }
}

/// Wraps a service-only factory with default in-flight tracking.
pub const fn in_flight_factory<Factory>(factory: Factory) -> InFlightFactory<Factory> {
    InFlightFactory::new(factory)
}

/// The underlying Tower balance type owned by [`DiscoveryBalance`].
pub type DiscoveryInner<Key, Backend, P, Factory, X = NoContext, O = IgnoreReadinessErrors> =
    Balance<
        Discovered<Key, Backend>,
        <Factory as EndpointFactory<Key, Backend>>::Service,
        P,
        X,
        <Factory as EndpointFactory<Key, Backend>>::Load,
        O,
    >;

/// A Tower balancer reconciled from versioned discovery snapshots.
///
/// Applying a newer snapshot preserves endpoint state only when both stable key
/// and backend allocation are unchanged. Every required new generation is
/// staged before the live pool is mutated, so a factory error leaves the pool
/// and applied revision unchanged.
///
/// ```
/// use std::convert::Infallible;
/// use poise_core::{Backend, policy::RoundRobin};
/// use poise_discovery::Directory;
/// use poise_tower::{DiscoveryBalance, in_flight_factory};
///
/// let mut directory = Directory::new();
/// directory.upsert("west", Backend::new("http://west"))?;
/// let factory = in_flight_factory(|_key: &&str, backend: &Backend<&str>| {
///     Ok::<_, Infallible>(backend.id().to_string())
/// });
/// let mut balance = DiscoveryBalance::new(RoundRobin::new(), factory);
/// let report = balance.apply_snapshot(&directory.snapshot())?;
/// assert_eq!(report.added(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct DiscoveryBalance<Key, Backend, P, Factory, X = NoContext, O = IgnoreReadinessErrors>
where
    Factory: EndpointFactory<Key, Backend>,
{
    balance: DiscoveryInner<Key, Backend, P, Factory, X, O>,
    factory: Factory,
    revision: Option<Revision>,
}

impl<Key, Backend, P, Factory>
    DiscoveryBalance<Key, Backend, P, Factory, NoContext, IgnoreReadinessErrors>
where
    Factory: EndpointFactory<Key, Backend>,
{
    /// Creates an empty request-independent discovery balancer.
    pub fn new(policy: P, factory: Factory) -> Self {
        Self {
            balance: Balance::from_parts(Vec::new(), policy, NoContext),
            factory,
            revision: None,
        }
    }
}

impl<Key, Backend, P, Factory, X, O> DiscoveryBalance<Key, Backend, P, Factory, X, O>
where
    Factory: EndpointFactory<Key, Backend>,
{
    /// Applies a coherent snapshot in snapshot order.
    ///
    /// An equal revision is an idempotent no-op. A newer revision stages all
    /// service builds before committing any endpoint or revision changes.
    /// Factory side effects outside this value cannot be rolled back.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError`] for a stale revision, duplicate identity in
    /// either side of reconciliation, or endpoint factory failure.
    pub fn apply_snapshot(
        &mut self,
        snapshot: &Snapshot<Discovered<Key, Backend>>,
    ) -> Result<ReconcileReport, ReconcileError<Factory::Error>>
    where
        Key: Eq + Hash,
    {
        if let Some(current) = self.revision {
            if snapshot.revision() < current {
                return Err(ReconcileError::StaleRevision {
                    current,
                    attempted: snapshot.revision(),
                });
            }
            if snapshot.revision() == current {
                return Ok(ReconcileReport {
                    revision: current,
                    applied: false,
                    retained: self.balance.endpoints().len(),
                    added: 0,
                    rebuilt: 0,
                    removed: 0,
                    draining: self
                        .balance
                        .endpoints()
                        .iter()
                        .filter(|endpoint| {
                            endpoint.candidate().membership() == Membership::Draining
                        })
                        .count(),
                });
            }
        }

        let plan = self.plan(snapshot)?;
        let mut staged = Vec::with_capacity(snapshot.len());
        for (index, (member, kind)) in snapshot.iter().zip(plan.kinds.iter()).enumerate() {
            if *kind == Planned::Retained {
                staged.push(None);
                continue;
            }
            let (service, load) = self
                .factory
                .build(member.key(), member.backend())
                .map_err(|source| ReconcileError::Build { index, source })?;
            staged.push(Some(Endpoint::with_tracker(member.clone(), service, load)));
        }

        let old_len = self.balance.endpoints.len();
        let target_by_old: HashMap<_, _> = plan
            .reuse
            .iter()
            .enumerate()
            .filter_map(|(new_index, old_index)| old_index.map(|old_index| (old_index, new_index)))
            .collect();
        let mut slots: Vec<Option<_>> = (0..snapshot.len()).map(|_| None).collect();
        for (old_index, mut endpoint) in std::mem::take(&mut self.balance.endpoints)
            .into_iter()
            .enumerate()
        {
            if let Some(&new_index) = target_by_old.get(&old_index) {
                if let Some(member) = snapshot.get(new_index) {
                    endpoint.candidate = member.clone();
                    slots[new_index] = Some(endpoint);
                }
            }
        }
        for (index, endpoint) in staged.into_iter().enumerate() {
            if endpoint.is_some() {
                slots[index] = endpoint;
            }
        }
        let endpoints = slots.into_iter().flatten().collect();

        let retained = plan
            .kinds
            .iter()
            .filter(|kind| **kind == Planned::Retained)
            .count();
        let added = plan
            .kinds
            .iter()
            .filter(|kind| **kind == Planned::Added)
            .count();
        let rebuilt = plan
            .kinds
            .iter()
            .filter(|kind| **kind == Planned::Rebuilt)
            .count();
        let removed = old_len.saturating_sub(retained + rebuilt);
        let draining = snapshot
            .iter()
            .filter(|member| member.membership() == Membership::Draining)
            .count();

        self.balance.endpoints = endpoints;
        self.revision = Some(snapshot.revision());
        Ok(ReconcileReport {
            revision: snapshot.revision(),
            applied: true,
            retained,
            added,
            rebuilt,
            removed,
            draining,
        })
    }

    /// Loads and applies the snapshot currently visible through a reader.
    ///
    /// # Errors
    ///
    /// Returns the same reconciliation errors as [`apply_snapshot`](Self::apply_snapshot).
    pub fn sync(
        &mut self,
        reader: &SnapshotReader<Discovered<Key, Backend>>,
    ) -> Result<ReconcileReport, ReconcileError<Factory::Error>>
    where
        Key: Eq + Hash,
    {
        self.apply_snapshot(&reader.load())
    }

    /// Returns the most recently applied revision.
    #[must_use]
    pub const fn revision(&self) -> Option<Revision> {
        self.revision
    }

    /// Returns the underlying Tower balancer.
    #[must_use]
    pub const fn balance(&self) -> &DiscoveryInner<Key, Backend, P, Factory, X, O> {
        &self.balance
    }

    /// Returns mutable access to the Tower balancer.
    ///
    /// Changing endpoint identities can cause the next reconciliation to reject
    /// duplicate live keys. Service readiness resets and policy changes are safe.
    pub const fn balance_mut(&mut self) -> &mut DiscoveryInner<Key, Backend, P, Factory, X, O> {
        &mut self.balance
    }

    /// Returns the endpoint factory.
    #[must_use]
    pub const fn factory(&self) -> &Factory {
        &self.factory
    }

    /// Returns mutable access to the endpoint factory.
    pub const fn factory_mut(&mut self) -> &mut Factory {
        &mut self.factory
    }

    /// Replaces the request-context projection without disturbing reconciled
    /// endpoint state.
    pub fn with_context<Y>(self, context: Y) -> DiscoveryBalance<Key, Backend, P, Factory, Y, O> {
        DiscoveryBalance {
            balance: self.balance.with_context(context),
            factory: self.factory,
            revision: self.revision,
        }
    }

    /// Replaces the readiness-error observer without disturbing reconciled
    /// endpoint state.
    pub fn with_readiness_observer<Y>(
        self,
        observer: Y,
    ) -> DiscoveryBalance<Key, Backend, P, Factory, X, Y> {
        DiscoveryBalance {
            balance: self.balance.with_readiness_observer(observer),
            factory: self.factory,
            revision: self.revision,
        }
    }

    fn plan(
        &self,
        snapshot: &Snapshot<Discovered<Key, Backend>>,
    ) -> Result<ReconcilePlan, ReconcileError<Factory::Error>>
    where
        Key: Eq + Hash,
    {
        let mut old_positions = HashMap::with_capacity(self.balance.endpoints().len());
        for (index, endpoint) in self.balance.endpoints().iter().enumerate() {
            if let Some(first) = old_positions.insert(endpoint.candidate().key(), index) {
                return Err(ReconcileError::DuplicateEndpointIdentity {
                    first,
                    second: index,
                });
            }
        }

        let mut snapshot_positions = HashSet::with_capacity(snapshot.len());
        let mut reuse = Vec::with_capacity(snapshot.len());
        let mut kinds = Vec::with_capacity(snapshot.len());
        for (index, member) in snapshot.iter().enumerate() {
            if !snapshot_positions.insert(member.key()) {
                let first = snapshot[..index]
                    .iter()
                    .position(|candidate| candidate.key() == member.key())
                    .unwrap_or(index);
                return Err(ReconcileError::DuplicateSnapshotIdentity {
                    first,
                    second: index,
                });
            }

            match old_positions.get(member.key()).copied() {
                Some(old_index)
                    if same_backend(self.balance.endpoints()[old_index].candidate(), member) =>
                {
                    reuse.push(Some(old_index));
                    kinds.push(Planned::Retained);
                }
                Some(_) => {
                    reuse.push(None);
                    kinds.push(Planned::Rebuilt);
                }
                None => {
                    reuse.push(None);
                    kinds.push(Planned::Added);
                }
            }
        }
        Ok(ReconcilePlan { reuse, kinds })
    }
}

impl<Request, Key, Backend, P, Factory, X, O> Service<Request>
    for DiscoveryBalance<Key, Backend, P, Factory, X, O>
where
    Backend: Candidate,
    Factory: EndpointFactory<Key, Backend>,
    Factory::Service: Service<Request>,
    P: Policy<Endpoint<Discovered<Key, Backend>, Factory::Service, Factory::Load>, X::Context>,
    X: RequestContext<Request>,
    O: ObserveReadinessError<<Factory::Service as Service<Request>>::Error>,
{
    type Response = <Factory::Service as Service<Request>>::Response;
    type Error = BalanceError<<Factory::Service as Service<Request>>::Error>;
    type Future = ResponseFuture<
        <Factory::Service as Service<Request>>::Future,
        <Factory::Load as LoadTracker>::Guard,
        <Factory::Service as Service<Request>>::Error,
    >;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.balance.poll_ready(context)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        self.balance.call(request)
    }
}

impl<Key, Backend, P, Factory, X, O> fmt::Debug for DiscoveryBalance<Key, Backend, P, Factory, X, O>
where
    Key: fmt::Debug,
    Backend: fmt::Debug,
    P: fmt::Debug,
    Factory: EndpointFactory<Key, Backend> + fmt::Debug,
    Factory::Service: fmt::Debug,
    Factory::Load: fmt::Debug,
    X: fmt::Debug,
    O: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveryBalance")
            .field("balance", &self.balance)
            .field("factory", &self.factory)
            .field("revision", &self.revision)
            .finish()
    }
}

fn same_backend<Key, Backend>(
    left: &Discovered<Key, Backend>,
    right: &Discovered<Key, Backend>,
) -> bool {
    std::sync::Arc::ptr_eq(&left.backend_arc(), &right.backend_arc())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Planned {
    Retained,
    Added,
    Rebuilt,
}

struct ReconcilePlan {
    reuse: Vec<Option<usize>>,
    kinds: Vec<Planned>,
}

/// Summary of one successful snapshot reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconcileReport {
    revision: Revision,
    applied: bool,
    retained: usize,
    added: usize,
    rebuilt: usize,
    removed: usize,
    draining: usize,
}

impl ReconcileReport {
    /// Returns the snapshot revision represented by the pool.
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    /// Returns whether a newer revision was committed.
    #[must_use]
    pub const fn applied(self) -> bool {
        self.applied
    }

    /// Returns generations whose service, load, and readiness were retained.
    #[must_use]
    pub const fn retained(self) -> usize {
        self.retained
    }

    /// Returns newly discovered identities that were built.
    #[must_use]
    pub const fn added(self) -> usize {
        self.added
    }

    /// Returns existing identities rebuilt for a new backend allocation.
    #[must_use]
    pub const fn rebuilt(self) -> usize {
        self.rebuilt
    }

    /// Returns old identities absent from the new snapshot.
    #[must_use]
    pub const fn removed(self) -> usize {
        self.removed
    }

    /// Returns members currently marked draining.
    #[must_use]
    pub const fn draining(self) -> usize {
        self.draining
    }
}

/// A snapshot could not be reconciled into the live Tower pool.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReconcileError<FactoryError> {
    /// The snapshot predates the applied revision.
    StaleRevision {
        /// Currently applied revision.
        current: Revision,
        /// Rejected revision.
        attempted: Revision,
    },
    /// Two existing endpoints advertise the same stable discovery identity.
    DuplicateEndpointIdentity {
        /// First endpoint index.
        first: usize,
        /// Duplicate endpoint index.
        second: usize,
    },
    /// Two snapshot members advertise the same stable discovery identity.
    DuplicateSnapshotIdentity {
        /// First member index.
        first: usize,
        /// Duplicate member index.
        second: usize,
    },
    /// Construction of a new endpoint generation failed.
    Build {
        /// Snapshot member index being built.
        index: usize,
        /// Factory error.
        source: FactoryError,
    },
}

impl<FactoryError: fmt::Display> fmt::Display for ReconcileError<FactoryError> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { current, attempted } => write!(
                f,
                "snapshot revision {attempted} predates applied revision {current}"
            ),
            Self::DuplicateEndpointIdentity { first, second } => write!(
                f,
                "live endpoints {first} and {second} share a discovery identity"
            ),
            Self::DuplicateSnapshotIdentity { first, second } => write!(
                f,
                "snapshot members {first} and {second} share a discovery identity"
            ),
            Self::Build { index, source } => {
                write!(f, "building snapshot member {index} failed: {source}")
            }
        }
    }
}

impl<FactoryError> Error for ReconcileError<FactoryError>
where
    FactoryError: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Build { source, .. } => Some(source),
            Self::StaleRevision { .. }
            | Self::DuplicateEndpointIdentity { .. }
            | Self::DuplicateSnapshotIdentity { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        future::{self, Future},
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use poise_core::{Backend as CoreBackend, policy::RoundRobin};
    use poise_discovery::{Directory, Effect, Snapshot, snapshot_channel};

    use crate::Readiness;

    use super::*;

    type Key = &'static str;
    type Backend = CoreBackend<&'static str, Spec>;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Spec {
        generation: usize,
        fail_build: bool,
    }

    fn backend(generation: usize) -> Backend {
        CoreBackend::new("transport").with_data(Spec {
            generation,
            fail_build: false,
        })
    }

    fn failing_backend(generation: usize) -> Backend {
        CoreBackend::new("transport").with_data(Spec {
            generation,
            fail_build: true,
        })
    }

    #[derive(Clone, Debug)]
    struct TestService {
        generation: usize,
        polls: Arc<AtomicUsize>,
    }

    impl Service<u64> for TestService {
        type Response = (usize, u64);
        type Error = Infallible;
        type Future = future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: u64) -> Self::Future {
            future::ready(Ok((self.generation, request)))
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct BuildError;

    impl fmt::Display for BuildError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("test endpoint build failed")
        }
    }

    impl Error for BuildError {}

    fn test_factory(
        builds: Arc<AtomicUsize>,
        service_drops: Arc<AtomicUsize>,
    ) -> impl EndpointFactory<
        Key,
        Backend,
        Service = DroppableService,
        Load = InFlight,
        Error = BuildError,
    > {
        in_flight_factory(move |_key: &Key, backend: &Backend| {
            builds.fetch_add(1, Ordering::Relaxed);
            if backend.data().fail_build {
                Err(BuildError)
            } else {
                Ok(DroppableService {
                    inner: TestService {
                        generation: backend.data().generation,
                        polls: Arc::new(AtomicUsize::new(0)),
                    },
                    drops: Arc::clone(&service_drops),
                })
            }
        })
    }

    #[derive(Debug)]
    struct DroppableService {
        inner: TestService,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DroppableService {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Service<u64> for DroppableService {
        type Response = (usize, u64);
        type Error = Infallible;
        type Future = future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(context)
        }

        fn call(&mut self, request: u64) -> Self::Future {
            self.inner.call(request)
        }
    }

    fn context() -> Context<'static> {
        Context::from_waker(Waker::noop())
    }

    fn poll_future<F: Future>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
        future.as_mut().poll(&mut context())
    }

    #[test]
    fn initial_snapshot_builds_in_snapshot_order_and_equal_revision_is_a_no_op() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        directory.upsert("b", backend(2)).unwrap();
        let builds = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let mut balance =
            DiscoveryBalance::new(RoundRobin::new(), test_factory(Arc::clone(&builds), drops));

        let snapshot = directory.snapshot();
        let report = balance.apply_snapshot(&snapshot).unwrap();
        assert_eq!(report.revision(), Revision::new(2));
        assert!(report.applied());
        assert_eq!(report.added(), 2);
        assert_eq!(report.retained(), 0);
        assert_eq!(builds.load(Ordering::Relaxed), 2);
        assert_eq!(balance.balance().endpoints()[0].candidate().key(), &"a");
        assert_eq!(balance.balance().endpoints()[1].candidate().key(), &"b");

        let repeated = balance.apply_snapshot(&snapshot).unwrap();
        assert!(!repeated.applied());
        assert_eq!(repeated.retained(), 2);
        assert_eq!(builds.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn drain_only_revision_preserves_service_load_and_readiness() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        directory.upsert("b", backend(2)).unwrap();
        let builds = Arc::new(AtomicUsize::new(0));
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::clone(&builds), Arc::new(AtomicUsize::new(0))),
        );
        balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let load = balance.balance().endpoints()[0].load_tracker().clone();
        let held = load.start().unwrap();
        let polls = Arc::clone(&balance.balance().endpoints()[0].service().inner.polls);

        directory.begin_drain("a").unwrap();
        let report = balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert_eq!(report.retained(), 2);
        assert_eq!(report.draining(), 1);
        assert_eq!(builds.load(Ordering::Relaxed), 2);
        assert_eq!(
            balance.balance().endpoints()[0].readiness(),
            Readiness::Ready
        );
        assert_eq!(
            balance.balance().endpoints()[0].candidate().membership(),
            Membership::Draining
        );
        assert_eq!(load.current(), 1);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let mut response = Box::pin(balance.call(9));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((2, 9))));
        assert_eq!(polls.load(Ordering::Relaxed), 1);
        drop(held);
    }

    #[test]
    fn same_key_with_new_backend_allocation_rebuilds_only_that_generation() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        directory.upsert("b", backend(2)).unwrap();
        let builds = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::clone(&builds), Arc::clone(&drops)),
        );
        balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let b_load = balance.balance().endpoints()[1].load_tracker().clone();

        directory.upsert("a", backend(3)).unwrap();
        let report = balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert_eq!(report.rebuilt(), 1);
        assert_eq!(report.retained(), 1);
        assert_eq!(report.added(), 0);
        assert_eq!(report.removed(), 0);
        assert_eq!(builds.load(Ordering::Relaxed), 3);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            balance.balance().endpoints()[0].readiness(),
            Readiness::Idle
        );
        assert_eq!(
            balance.balance().endpoints()[1].readiness(),
            Readiness::Ready
        );
        assert_eq!(
            balance.balance().endpoints()[1].load_tracker().current(),
            b_load.current()
        );
    }

    #[test]
    fn reorder_moves_retained_state_with_identity() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        directory.upsert("b", backend(2)).unwrap();
        let current = directory.snapshot();
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))),
        );
        balance.apply_snapshot(&current).unwrap();
        let a_load = balance.balance().endpoints()[0].load_tracker().clone();
        let held = a_load.start().unwrap();

        let reordered = Snapshot::new(
            Revision::new(current.revision().get() + 1),
            vec![current[1].clone(), current[0].clone()],
        );
        let report = balance.apply_snapshot(&reordered).unwrap();
        assert_eq!(report.retained(), 2);
        assert_eq!(balance.balance().endpoints()[0].candidate().key(), &"b");
        assert_eq!(balance.balance().endpoints()[1].candidate().key(), &"a");
        assert_eq!(balance.balance().endpoints()[1].load_tracker().current(), 1);
        drop(held);
    }

    #[test]
    fn factory_failure_leaves_live_pool_and_revision_unchanged() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let builds = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::clone(&builds), Arc::clone(&drops)),
        );
        balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let revision = balance.revision();

        directory.upsert("b", backend(2)).unwrap();
        directory.upsert("c", failing_backend(3)).unwrap();
        let error = balance.apply_snapshot(&directory.snapshot()).unwrap_err();
        assert_eq!(
            error,
            ReconcileError::Build {
                index: 2,
                source: BuildError
            }
        );
        assert_eq!(balance.revision(), revision);
        assert_eq!(balance.balance().endpoints().len(), 1);
        assert_eq!(balance.balance().endpoints()[0].candidate().key(), &"a");
        assert_eq!(
            balance.balance().endpoints()[0].readiness(),
            Readiness::Ready
        );
        assert_eq!(builds.load(Ordering::Relaxed), 3);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn stale_and_duplicate_snapshots_are_rejected_before_building() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let first = directory.snapshot();
        directory.upsert("b", backend(2)).unwrap();
        let second = directory.snapshot();
        let builds = Arc::new(AtomicUsize::new(0));
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::clone(&builds), Arc::new(AtomicUsize::new(0))),
        );
        balance.apply_snapshot(&second).unwrap();
        let built = builds.load(Ordering::Relaxed);

        assert_eq!(
            balance.apply_snapshot(&first).unwrap_err(),
            ReconcileError::StaleRevision {
                current: second.revision(),
                attempted: first.revision()
            }
        );
        let duplicate = Snapshot::new(
            Revision::new(second.revision().get() + 1),
            vec![second[0].clone(), second[0].clone()],
        );
        assert_eq!(
            balance.apply_snapshot(&duplicate).unwrap_err(),
            ReconcileError::DuplicateSnapshotIdentity {
                first: 0,
                second: 1
            }
        );
        assert_eq!(builds.load(Ordering::Relaxed), built);
        assert_eq!(balance.revision(), Some(second.revision()));
    }

    #[test]
    fn duplicate_live_identity_is_rejected_before_building() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let first = directory.snapshot();
        let builds = Arc::new(AtomicUsize::new(0));
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::clone(&builds), Arc::new(AtomicUsize::new(0))),
        );
        balance.apply_snapshot(&first).unwrap();
        let duplicate = Endpoint::new(
            first[0].clone(),
            DroppableService {
                inner: TestService {
                    generation: 99,
                    polls: Arc::new(AtomicUsize::new(0)),
                },
                drops: Arc::new(AtomicUsize::new(0)),
            },
        );
        balance.balance_mut().push(duplicate);
        let next = Snapshot::new(
            Revision::new(first.revision().get() + 1),
            vec![first[0].clone()],
        );
        let built = builds.load(Ordering::Relaxed);

        assert_eq!(
            balance.apply_snapshot(&next).unwrap_err(),
            ReconcileError::DuplicateEndpointIdentity {
                first: 0,
                second: 1
            }
        );
        assert_eq!(builds.load(Ordering::Relaxed), built);
    }

    #[derive(Debug)]
    struct PendingService {
        dropped: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct PendingResponse {
        dropped: Arc<AtomicUsize>,
    }

    impl Future for PendingResponse {
        type Output = Result<(), Infallible>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for PendingResponse {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Service<()> for PendingService {
        type Response = ();
        type Error = Infallible;
        type Future = PendingResponse;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, (): ()) -> Self::Future {
            PendingResponse {
                dropped: Arc::clone(&self.dropped),
            }
        }
    }

    #[test]
    fn physical_retirement_does_not_invalidate_an_outstanding_response() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let future_drops = Arc::new(AtomicUsize::new(0));
        let drop_handle = Arc::clone(&future_drops);
        let factory = in_flight_factory(move |_key: &Key, _backend: &Backend| {
            Ok::<_, Infallible>(PendingService {
                dropped: Arc::clone(&drop_handle),
            })
        });
        let mut balance = DiscoveryBalance::new(RoundRobin::new(), factory);
        balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let load = balance.balance().endpoints()[0].load_tracker().clone();
        let response = balance.call(());
        assert_eq!(load.current(), 1);

        directory.begin_drain("a").unwrap();
        balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert_eq!(balance.balance().endpoints().len(), 1);
        assert_eq!(
            balance.balance().endpoints()[0].candidate().membership(),
            Membership::Draining
        );
        assert_eq!(
            directory.finish_drain("a").unwrap().effect(),
            Effect::DrainFinished
        );
        let report = balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert_eq!(report.removed(), 1);
        assert!(balance.balance().endpoints().is_empty());
        assert_eq!(load.current(), 1);

        drop(response);
        assert_eq!(load.current(), 0);
        assert_eq!(future_drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_reader_sync_is_idempotent_and_tracks_publications() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (mut publisher, reader) = snapshot_channel(directory.snapshot());
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))),
        );

        assert!(balance.sync(&reader).unwrap().applied());
        assert!(!balance.sync(&reader).unwrap().applied());
        directory.upsert("b", backend(2)).unwrap();
        directory.publish(&mut publisher).unwrap();
        let report = balance.sync(&reader).unwrap();
        assert!(report.applied());
        assert_eq!(report.added(), 1);
        assert_eq!(balance.revision(), Some(Revision::new(2)));
    }

    #[test]
    fn initial_empty_revision_can_be_applied() {
        let directory: Directory<Key, Backend> = Directory::new();
        let mut balance = DiscoveryBalance::new(
            RoundRobin::new(),
            test_factory(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))),
        );

        let report = balance.apply_snapshot(&directory.snapshot()).unwrap();
        assert!(report.applied());
        assert_eq!(report.revision(), Revision::INITIAL);
        assert_eq!(balance.revision(), Some(Revision::INITIAL));
    }
}
