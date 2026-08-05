use std::{
    error::Error,
    fmt,
    future::Future,
    hash::Hash,
    marker::PhantomData,
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use poise_discovery::{Discovered, SnapshotStream};
use tower::Service;

use crate::{
    DiscoveryBalance, EndpointFactory, IgnoreReadinessErrors, NoContext, ReconcileError,
    ReconcileReport,
};

/// Behavior after the snapshot publisher is dropped.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum StreamEndPolicy {
    /// Continue serving the last successfully reconciled snapshot.
    #[default]
    LastKnownGood,
    /// Make aggregate readiness fail after the final snapshot is applied.
    FailClosed,
}

/// Stream polling configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StreamingConfig {
    max_updates_per_poll: NonZeroUsize,
    end_policy: StreamEndPolicy,
}

impl StreamingConfig {
    /// Creates a configuration with a fairness budget per `poll_ready` call.
    #[must_use]
    pub const fn new(max_updates_per_poll: NonZeroUsize) -> Self {
        Self {
            max_updates_per_poll,
            end_policy: StreamEndPolicy::LastKnownGood,
        }
    }

    /// Sets behavior after the snapshot publisher closes.
    #[must_use]
    pub const fn with_end_policy(mut self, policy: StreamEndPolicy) -> Self {
        self.end_policy = policy;
        self
    }

    /// Returns the maximum snapshots applied by one readiness poll.
    #[must_use]
    pub const fn max_updates_per_poll(self) -> NonZeroUsize {
        self.max_updates_per_poll
    }

    /// Returns the publisher-close behavior.
    #[must_use]
    pub const fn end_policy(self) -> StreamEndPolicy {
        self.end_policy
    }
}

/// Owned components returned by [`StreamingDiscoveryBalance::into_parts`].
pub type StreamingParts<Key, Backend, P, Factory, X, O> = (
    DiscoveryBalance<Key, Backend, P, Factory, X, O>,
    SnapshotStream<Discovered<Key, Backend>>,
    StreamingConfig,
);

impl Default for StreamingConfig {
    fn default() -> Self {
        Self::new(NonZeroUsize::new(16).expect("sixteen is non-zero"))
    }
}

/// A [`DiscoveryBalance`] driven by a coalescing snapshot stream.
///
/// `poll_ready` first applies every immediately visible snapshot up to the
/// configured fairness budget, then delegates to the reconciled Tower pool.
/// It registers both discovery and service readiness wakers without spawning a
/// task or choosing an executor.
pub struct StreamingDiscoveryBalance<
    Key,
    Backend,
    P,
    Factory,
    X = NoContext,
    O = IgnoreReadinessErrors,
> where
    Factory: EndpointFactory<Key, Backend>,
{
    inner: DiscoveryBalance<Key, Backend, P, Factory, X, O>,
    updates: SnapshotStream<Discovered<Key, Backend>>,
    config: StreamingConfig,
    updates_closed: bool,
    last_report: Option<ReconcileReport>,
}

impl<Key, Backend, P, Factory>
    StreamingDiscoveryBalance<Key, Backend, P, Factory, NoContext, IgnoreReadinessErrors>
where
    Factory: EndpointFactory<Key, Backend>,
{
    /// Creates a streaming balancer that initially yields the reader's current
    /// snapshot.
    pub fn new(
        policy: P,
        factory: Factory,
        reader: &poise_discovery::SnapshotReader<Discovered<Key, Backend>>,
    ) -> Self {
        Self::from_parts(
            DiscoveryBalance::new(policy, factory),
            reader.subscribe(),
            StreamingConfig::default(),
        )
    }
}

impl<Key, Backend, P, Factory, X, O> StreamingDiscoveryBalance<Key, Backend, P, Factory, X, O>
where
    Factory: EndpointFactory<Key, Backend>,
{
    /// Creates a streaming wrapper from explicit reconciler, subscription, and
    /// configuration state.
    pub const fn from_parts(
        inner: DiscoveryBalance<Key, Backend, P, Factory, X, O>,
        updates: SnapshotStream<Discovered<Key, Backend>>,
        config: StreamingConfig,
    ) -> Self {
        Self {
            inner,
            updates,
            config,
            updates_closed: false,
            last_report: None,
        }
    }

    /// Polls and reconciles currently visible discovery state.
    ///
    /// Returns `Pending` only when the fairness budget was exhausted; the
    /// current task is self-woken so reconciliation continues. A settled stream
    /// returns the number of snapshots applied during this call.
    ///
    /// # Errors
    ///
    /// Returns a reconciliation error without mutating the last good pool.
    pub fn poll_updates(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<usize, ReconcileError<Factory::Error>>>
    where
        Key: Eq + Hash,
    {
        if self.updates_closed {
            return Poll::Ready(Ok(0));
        }

        let mut applied = 0;
        for _ in 0..self.config.max_updates_per_poll.get() {
            match self.updates.poll_snapshot(context) {
                Poll::Pending => return Poll::Ready(Ok(applied)),
                Poll::Ready(None) => {
                    self.updates_closed = true;
                    return Poll::Ready(Ok(applied));
                }
                Poll::Ready(Some(snapshot)) => {
                    let report = self.inner.apply_snapshot(&snapshot)?;
                    self.last_report = Some(report);
                    applied += 1;
                }
            }
        }

        context.waker().wake_by_ref();
        Poll::Pending
    }

    /// Returns the reconciled Tower balancer.
    #[must_use]
    pub const fn inner(&self) -> &DiscoveryBalance<Key, Backend, P, Factory, X, O> {
        &self.inner
    }

    /// Returns mutable access to the reconciled Tower balancer.
    pub const fn inner_mut(&mut self) -> &mut DiscoveryBalance<Key, Backend, P, Factory, X, O> {
        &mut self.inner
    }

    /// Returns the snapshot subscription.
    #[must_use]
    pub const fn updates(&self) -> &SnapshotStream<Discovered<Key, Backend>> {
        &self.updates
    }

    /// Returns the stream configuration.
    #[must_use]
    pub const fn config(&self) -> StreamingConfig {
        self.config
    }

    /// Returns whether the publisher has closed.
    #[must_use]
    pub const fn updates_closed(&self) -> bool {
        self.updates_closed
    }

    /// Returns the report from the most recently observed snapshot.
    #[must_use]
    pub const fn last_report(&self) -> Option<ReconcileReport> {
        self.last_report
    }

    /// Decomposes the wrapper into reconciler, subscription, and configuration.
    pub fn into_parts(self) -> StreamingParts<Key, Backend, P, Factory, X, O> {
        (self.inner, self.updates, self.config)
    }
}

impl<Request, Key, Backend, P, Factory, X, O> Service<Request>
    for StreamingDiscoveryBalance<Key, Backend, P, Factory, X, O>
where
    Key: Eq + Hash,
    Factory: EndpointFactory<Key, Backend>,
    DiscoveryBalance<Key, Backend, P, Factory, X, O>: Service<Request>,
{
    type Response =
        <DiscoveryBalance<Key, Backend, P, Factory, X, O> as Service<Request>>::Response;
    type Error = StreamingError<
        <DiscoveryBalance<Key, Backend, P, Factory, X, O> as Service<Request>>::Error,
        Factory::Error,
    >;
    type Future = StreamingResponseFuture<
        <DiscoveryBalance<Key, Backend, P, Factory, X, O> as Service<Request>>::Future,
        Factory::Error,
    >;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.poll_updates(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(source)) => {
                return Poll::Ready(Err(StreamingError::Reconcile(source)));
            }
            Poll::Ready(Ok(_)) => {}
        }
        if self.updates_closed && self.config.end_policy == StreamEndPolicy::FailClosed {
            return Poll::Ready(Err(StreamingError::UpdatesClosed));
        }
        self.inner
            .poll_ready(context)
            .map_err(StreamingError::Dispatch)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        StreamingResponseFuture::new(self.inner.call(request))
    }
}

impl<Key, Backend, P, Factory, X, O> fmt::Debug
    for StreamingDiscoveryBalance<Key, Backend, P, Factory, X, O>
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
        f.debug_struct("StreamingDiscoveryBalance")
            .field("inner", &self.inner)
            .field("updates", &self.updates)
            .field("config", &self.config)
            .field("updates_closed", &self.updates_closed)
            .field("last_report", &self.last_report)
            .finish()
    }
}

pin_project! {
    /// Response future returned by [`StreamingDiscoveryBalance`].
    pub struct StreamingResponseFuture<F, FactoryError> {
        #[pin]
        inner: F,
        marker: PhantomData<fn() -> FactoryError>,
    }
}

impl<F, FactoryError> StreamingResponseFuture<F, FactoryError> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            marker: PhantomData,
        }
    }
}

impl<F, Response, DispatchError, FactoryError> Future for StreamingResponseFuture<F, FactoryError>
where
    F: Future<Output = Result<Response, DispatchError>>,
{
    type Output = Result<Response, StreamingError<DispatchError, FactoryError>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.project()
            .inner
            .poll(context)
            .map_err(StreamingError::Dispatch)
    }
}

/// Streaming discovery, reconciliation, or endpoint dispatch failure.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StreamingError<DispatchError, FactoryError> {
    /// The reconciled Tower pool failed readiness or dispatch.
    Dispatch(DispatchError),
    /// A snapshot could not be reconciled.
    Reconcile(ReconcileError<FactoryError>),
    /// The publisher closed while fail-closed behavior was configured.
    UpdatesClosed,
}

impl<DispatchError, FactoryError> fmt::Display for StreamingError<DispatchError, FactoryError>
where
    DispatchError: fmt::Display,
    FactoryError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dispatch(source) => write!(f, "Tower dispatch failed: {source}"),
            Self::Reconcile(source) => write!(f, "discovery reconciliation failed: {source}"),
            Self::UpdatesClosed => f.write_str("the discovery snapshot publisher closed"),
        }
    }
}

impl<DispatchError, FactoryError> Error for StreamingError<DispatchError, FactoryError>
where
    DispatchError: Error + 'static,
    FactoryError: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dispatch(source) => Some(source),
            Self::Reconcile(source) => Some(source),
            Self::UpdatesClosed => None,
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
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
    };

    use poise_core::{Backend as CoreBackend, PickError, policy::RoundRobin};
    use poise_discovery::{Directory, Revision, SnapshotPublisher, snapshot_channel};

    use crate::{BalanceError, in_flight_factory};

    use super::*;

    type Key = &'static str;
    type Backend = CoreBackend<&'static str, Spec>;
    type PublisherSlot = Arc<Mutex<Option<SnapshotPublisher<Discovered<Key, Backend>>>>>;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Spec {
        generation: usize,
        fail: bool,
    }

    fn backend(generation: usize) -> Backend {
        CoreBackend::new("transport").with_data(Spec {
            generation,
            fail: false,
        })
    }

    fn failing_backend(generation: usize) -> Backend {
        CoreBackend::new("transport").with_data(Spec {
            generation,
            fail: true,
        })
    }

    #[derive(Debug)]
    struct TestService {
        generation: usize,
    }

    impl Service<u64> for TestService {
        type Response = (usize, u64);
        type Error = Infallible;
        type Future = future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
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
            f.write_str("streaming test build failed")
        }
    }

    impl Error for BuildError {}

    fn factory(
        builds: Arc<AtomicUsize>,
    ) -> impl EndpointFactory<
        Key,
        Backend,
        Service = TestService,
        Load = poise_core::InFlight,
        Error = BuildError,
    > {
        in_flight_factory(move |_key: &Key, backend: &Backend| {
            builds.fetch_add(1, Ordering::Relaxed);
            if backend.data().fail {
                Err(BuildError)
            } else {
                Ok(TestService {
                    generation: backend.data().generation,
                })
            }
        })
    }

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn context() -> Context<'static> {
        Context::from_waker(Waker::noop())
    }

    fn poll_future<F: Future>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
        future.as_mut().poll(&mut context())
    }

    #[test]
    fn first_readiness_poll_applies_current_snapshot_before_dispatch() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (_publisher, reader) = snapshot_channel(directory.snapshot());
        let builds = Arc::new(AtomicUsize::new(0));
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::clone(&builds)),
            &reader,
        );

        assert_eq!(balance.inner().revision(), None);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(balance.inner().revision(), Some(directory.revision()));
        assert_eq!(balance.last_report().map(ReconcileReport::added), Some(1));
        assert_eq!(builds.load(Ordering::Relaxed), 1);

        let mut response = Box::pin(balance.call(7));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((1, 7))));
    }

    #[test]
    fn published_generation_is_reconciled_before_the_next_call() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (mut publisher, reader) = snapshot_channel(directory.snapshot());
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::new(AtomicUsize::new(0))),
            &reader,
        );
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        directory.upsert("a", backend(2)).unwrap();
        directory.publish(&mut publisher).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(balance.last_report().map(ReconcileReport::rebuilt), Some(1));
        let mut response = Box::pin(balance.call(9));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((2, 9))));
    }

    #[test]
    fn burst_publications_build_only_the_latest_generation() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (mut publisher, reader) = snapshot_channel(directory.snapshot());
        let builds = Arc::new(AtomicUsize::new(0));
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::clone(&builds)),
            &reader,
        );
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        directory.upsert("a", backend(2)).unwrap();
        directory.publish(&mut publisher).unwrap();
        directory.upsert("a", backend(3)).unwrap();
        directory.publish(&mut publisher).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(builds.load(Ordering::Relaxed), 2);
        assert_eq!(balance.inner().revision(), Some(Revision::new(3)));
        let mut response = Box::pin(balance.call(0));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((3, 0))));
    }

    #[test]
    fn update_build_failure_preserves_and_can_resume_last_good_pool() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (mut publisher, reader) = snapshot_channel(directory.snapshot());
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::new(AtomicUsize::new(0))),
            &reader,
        );
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let good_revision = balance.inner().revision();

        directory.upsert("a", failing_backend(2)).unwrap();
        directory.publish(&mut publisher).unwrap();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Err(StreamingError::Reconcile(ReconcileError::Build {
                index: 0,
                source: BuildError
            })))
        ));
        assert_eq!(balance.inner().revision(), good_revision);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let mut response = Box::pin(balance.call(3));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((1, 3))));
    }

    #[test]
    fn publisher_close_supports_last_known_good_and_fail_closed_modes() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();

        let (publisher, reader) = snapshot_channel(directory.snapshot());
        let mut available = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::new(AtomicUsize::new(0))),
            &reader,
        );
        assert!(matches!(
            available.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        drop(publisher);
        assert!(matches!(
            available.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert!(available.updates_closed());

        let (publisher, reader) = snapshot_channel(directory.snapshot());
        let inner =
            DiscoveryBalance::new(RoundRobin::new(), factory(Arc::new(AtomicUsize::new(0))));
        let config = StreamingConfig::default().with_end_policy(StreamEndPolicy::FailClosed);
        let mut closed = StreamingDiscoveryBalance::from_parts(inner, reader.subscribe(), config);
        assert!(matches!(
            closed.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        drop(publisher);
        assert!(matches!(
            closed.poll_ready(&mut context()),
            Poll::Ready(Err(StreamingError::UpdatesClosed))
        ));
    }

    #[test]
    fn discovery_publication_wakes_a_ready_tower_service() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let (mut publisher, reader) = snapshot_channel(directory.snapshot());
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::new(AtomicUsize::new(0))),
            &reader,
        );
        let wake_count = Arc::new(WakeCount::default());
        let waker = Waker::from(Arc::clone(&wake_count));
        assert!(matches!(
            balance.poll_ready(&mut Context::from_waker(&waker)),
            Poll::Ready(Ok(()))
        ));

        directory.upsert("a", backend(2)).unwrap();
        directory.publish(&mut publisher).unwrap();
        assert_eq!(wake_count.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn fairness_budget_self_wakes_when_factory_publishes_during_reconcile() {
        let mut directory = Directory::new();
        directory.upsert("a", backend(1)).unwrap();
        let first = directory.snapshot();
        directory.upsert("a", backend(2)).unwrap();
        let second = directory.snapshot();
        let (publisher, reader) = snapshot_channel(first);
        let publisher: PublisherSlot = Arc::new(Mutex::new(Some(publisher)));
        let publish_once = Arc::clone(&publisher);
        let mut second = Some(second);
        let factory = in_flight_factory(move |_key: &Key, backend: &Backend| {
            if backend.data().generation == 1 {
                let mut publisher = publish_once.lock().unwrap();
                publisher
                    .as_mut()
                    .unwrap()
                    .publish(second.take().unwrap())
                    .unwrap();
            }
            Ok::<_, Infallible>(TestService {
                generation: backend.data().generation,
            })
        });
        let inner = DiscoveryBalance::new(RoundRobin::new(), factory);
        let config = StreamingConfig::new(NonZeroUsize::MIN);
        let mut balance = StreamingDiscoveryBalance::from_parts(inner, reader.subscribe(), config);
        let wake_count = Arc::new(WakeCount::default());
        let waker = Waker::from(Arc::clone(&wake_count));

        assert!(matches!(
            balance.poll_ready(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        assert!(wake_count.0.load(Ordering::Relaxed) >= 1);
        assert!(matches!(
            balance.poll_ready(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        assert_eq!(balance.inner().revision(), Some(Revision::new(2)));

        // The next poll reaches the stream's settled state and the ready pool.
        assert!(matches!(
            balance.poll_ready(&mut Context::from_waker(&waker)),
            Poll::Ready(Ok(()))
        ));
    }

    #[test]
    fn call_errors_are_wrapped_as_dispatch_errors() {
        let directory: Directory<Key, Backend> = Directory::new();
        let (_publisher, reader) = snapshot_channel(directory.snapshot());
        let mut balance = StreamingDiscoveryBalance::new(
            RoundRobin::new(),
            factory(Arc::new(AtomicUsize::new(0))),
            &reader,
        );
        let mut response = Box::pin(balance.call(1));

        assert!(matches!(
            poll_future(&mut response),
            Poll::Ready(Err(StreamingError::Dispatch(BalanceError::Selection(
                PickError::Empty
            ))))
        ));
    }
}
