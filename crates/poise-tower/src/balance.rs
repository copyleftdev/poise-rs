use std::task::{Context, Poll};

use poise_core::{Candidate, InFlight, Policy};
use tower::Service;

use crate::{
    BalanceError, Endpoint, IgnoreReadinessErrors, LoadTracker, NoContext, ObserveReadinessError,
    Readiness, RequestContext, ResponseFuture,
};

/// A policy-driven Tower service over a fixed endpoint set.
///
/// `poll_ready` polls every eligible idle service, retaining successful
/// readiness reservations so the policy can choose across the complete ready
/// set. A readiness failure quarantines only that endpoint; call
/// [`Endpoint::reset_readiness`] through [`endpoints_mut`](Self::endpoints_mut)
/// to retry it.
pub struct Balance<C, S, P, X = NoContext, L = InFlight, O = IgnoreReadinessErrors> {
    pub(crate) endpoints: Vec<Endpoint<C, S, L>>,
    policy: P,
    context: X,
    readiness_observer: O,
}

impl<C, S, P> Balance<C, S, P, NoContext, InFlight> {
    /// Creates a request-independent balancer with in-flight load tracking.
    pub fn new(endpoints: Vec<Endpoint<C, S>>, policy: P) -> Self {
        Self::from_parts(endpoints, policy, NoContext)
    }
}

impl<C, S, P, X, L> Balance<C, S, P, X, L, IgnoreReadinessErrors> {
    /// Creates a balancer from explicit endpoints, policy, and request-context
    /// projection.
    pub const fn from_parts(endpoints: Vec<Endpoint<C, S, L>>, policy: P, context: X) -> Self {
        Self {
            endpoints,
            policy,
            context,
            readiness_observer: IgnoreReadinessErrors,
        }
    }
}

impl<C, S, P, X, L, O> Balance<C, S, P, X, L, O> {
    /// Replaces the request-context projection without changing endpoint or
    /// policy state.
    pub fn with_context<Y>(self, context: Y) -> Balance<C, S, P, Y, L, O> {
        Balance {
            endpoints: self.endpoints,
            policy: self.policy,
            context,
            readiness_observer: self.readiness_observer,
        }
    }

    /// Replaces the readiness-error observer without changing dispatch state.
    pub fn with_readiness_observer<Y>(self, observer: Y) -> Balance<C, S, P, X, L, Y> {
        Balance {
            endpoints: self.endpoints,
            policy: self.policy,
            context: self.context,
            readiness_observer: observer,
        }
    }

    /// Returns the endpoint slice.
    #[must_use]
    pub fn endpoints(&self) -> &[Endpoint<C, S, L>] {
        &self.endpoints
    }

    /// Returns mutable endpoints for membership updates or readiness resets.
    ///
    /// Mutating a service through [`Endpoint::service_mut`] automatically
    /// invalidates its retained readiness reservation.
    pub fn endpoints_mut(&mut self) -> &mut [Endpoint<C, S, L>] {
        &mut self.endpoints
    }

    /// Adds an idle endpoint to the end of the pool.
    pub fn push(&mut self, mut endpoint: Endpoint<C, S, L>) {
        endpoint.reset_readiness();
        self.endpoints.push(endpoint);
    }

    /// Removes and returns one endpoint.
    pub fn remove(&mut self, index: usize) -> Option<Endpoint<C, S, L>> {
        if index < self.endpoints.len() {
            Some(self.endpoints.remove(index))
        } else {
            None
        }
    }

    /// Returns the selection policy.
    #[must_use]
    pub const fn policy(&self) -> &P {
        &self.policy
    }

    /// Returns mutable access to the selection policy.
    pub const fn policy_mut(&mut self) -> &mut P {
        &mut self.policy
    }

    /// Returns the request-context projection.
    #[must_use]
    pub const fn context(&self) -> &X {
        &self.context
    }

    /// Returns the readiness-error observer.
    #[must_use]
    pub const fn readiness_observer(&self) -> &O {
        &self.readiness_observer
    }

    /// Decomposes the balancer into endpoints, policy, context projection, and
    /// readiness observer.
    pub fn into_parts(self) -> (Vec<Endpoint<C, S, L>>, P, X, O) {
        (
            self.endpoints,
            self.policy,
            self.context,
            self.readiness_observer,
        )
    }
}

impl<C, S, P, X, L, O> std::fmt::Debug for Balance<C, S, P, X, L, O>
where
    C: std::fmt::Debug,
    S: std::fmt::Debug,
    P: std::fmt::Debug,
    X: std::fmt::Debug,
    L: std::fmt::Debug,
    O: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Balance")
            .field("endpoints", &self.endpoints)
            .field("policy", &self.policy)
            .field("context", &self.context)
            .field("readiness_observer", &self.readiness_observer)
            .finish()
    }
}

impl<Request, C, S, P, X, L, O> Service<Request> for Balance<C, S, P, X, L, O>
where
    C: Candidate,
    S: Service<Request>,
    P: Policy<Endpoint<C, S, L>, X::Context>,
    X: RequestContext<Request>,
    L: LoadTracker,
    O: ObserveReadinessError<S::Error>,
{
    type Response = S::Response;
    type Error = BalanceError<S::Error>;
    type Future = ResponseFuture<S::Future, L::Guard, S::Error>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.endpoints.is_empty() {
            return Poll::Ready(Err(BalanceError::NoEndpoints));
        }

        let mut has_eligible = false;
        let mut has_ready = false;
        let mut has_pending = false;
        let mut readiness_error = None;

        for (index, endpoint) in self.endpoints.iter_mut().enumerate() {
            if !endpoint.candidate.is_eligible() {
                continue;
            }
            has_eligible = true;

            match endpoint.readiness {
                Readiness::Ready => has_ready = true,
                Readiness::Failed => {}
                Readiness::Idle => match endpoint.service.poll_ready(context) {
                    Poll::Pending => has_pending = true,
                    Poll::Ready(Ok(())) => {
                        endpoint.readiness = Readiness::Ready;
                        has_ready = true;
                    }
                    Poll::Ready(Err(source)) => {
                        endpoint.readiness = Readiness::Failed;
                        self.readiness_observer.observe(index, &source);
                        readiness_error = Some(BalanceError::Endpoint { index, source });
                    }
                },
            }
        }

        if has_ready {
            Poll::Ready(Ok(()))
        } else if has_pending {
            Poll::Pending
        } else if let Some(error) = readiness_error {
            Poll::Ready(Err(error))
        } else if has_eligible {
            Poll::Ready(Err(BalanceError::NoReadyEndpoints))
        } else {
            Poll::Ready(Err(BalanceError::NoEligibleEndpoints))
        }
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let selection = match self
            .policy
            .pick(&self.endpoints, self.context.context(&request))
        {
            Ok(selection) => selection,
            Err(error) => return ResponseFuture::failed(BalanceError::Selection(error)),
        };
        let index = selection.index();
        let len = self.endpoints.len();
        let Some(endpoint) = self.endpoints.get_mut(index) else {
            return ResponseFuture::failed(BalanceError::InvalidSelection { index, len });
        };
        let guard = match endpoint.load.start() {
            Ok(guard) => guard,
            Err(source) => {
                return ResponseFuture::failed(BalanceError::AtCapacity { index, source });
            }
        };

        endpoint.readiness = Readiness::Idle;
        let future = endpoint.service.call(request);
        ResponseFuture::running(future, guard, index)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        error::Error,
        fmt,
        future::{self, Future},
        num::NonZeroU64,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use poise_core::{
        AtCapacity, Backend, Candidate, InFlight, LoadMetric, PickError, Policy,
        policy::{LeastLoaded, RoundRobin},
    };

    use crate::{LoadGuard, UseRequest};

    use super::*;

    const PENDING: u8 = 0;
    const READY: u8 = 1;
    const FAILED: u8 = 2;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Readiness,
        Response,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Readiness => f.write_str("readiness failed"),
                Self::Response => f.write_str("response failed"),
            }
        }
    }

    impl Error for TestError {}

    #[derive(Clone, Debug)]
    struct TestService {
        id: usize,
        mode: Arc<AtomicU8>,
        response_error: Arc<AtomicBool>,
        polls: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
    }

    impl TestService {
        fn new(id: usize, mode: u8) -> Self {
            Self {
                id,
                mode: Arc::new(AtomicU8::new(mode)),
                response_error: Arc::new(AtomicBool::new(false)),
                polls: Arc::new(AtomicUsize::new(0)),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl Service<u64> for TestService {
        type Response = (usize, u64);
        type Error = TestError;
        type Future = future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            match self.mode.load(Ordering::Relaxed) {
                PENDING => Poll::Pending,
                READY => Poll::Ready(Ok(())),
                FAILED => Poll::Ready(Err(TestError::Readiness)),
                value => panic!("unexpected test readiness mode {value}"),
            }
        }

        fn call(&mut self, request: u64) -> Self::Future {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.response_error.load(Ordering::Relaxed) {
                future::ready(Err(TestError::Response))
            } else {
                future::ready(Ok((self.id, request)))
            }
        }
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

    #[derive(Debug, Default)]
    struct RecordingState {
        active: AtomicUsize,
        completed: AtomicUsize,
        cancelled: AtomicUsize,
    }

    #[derive(Clone, Debug, Default)]
    struct RecordingTracker {
        state: Arc<RecordingState>,
    }

    impl LoadMetric for RecordingTracker {
        type Metric = usize;

        fn measure(&self) -> Self::Metric {
            self.state.active.load(Ordering::Relaxed)
        }
    }

    impl LoadTracker for RecordingTracker {
        type Guard = RecordingGuard;

        fn start(&self) -> Result<Self::Guard, AtCapacity> {
            self.state.active.fetch_add(1, Ordering::Relaxed);
            Ok(RecordingGuard {
                state: Arc::clone(&self.state),
                finished: false,
            })
        }
    }

    #[derive(Debug)]
    struct RecordingGuard {
        state: Arc<RecordingState>,
        finished: bool,
    }

    impl LoadGuard for RecordingGuard {
        fn complete(mut self) {
            self.state.active.fetch_sub(1, Ordering::Relaxed);
            self.state.completed.fetch_add(1, Ordering::Relaxed);
            self.finished = true;
        }
    }

    impl Drop for RecordingGuard {
        fn drop(&mut self) {
            if !self.finished {
                self.state.active.fetch_sub(1, Ordering::Relaxed);
                self.state.cancelled.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn context() -> Context<'static> {
        Context::from_waker(Waker::noop())
    }

    fn poll_future<F: Future>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
        future.as_mut().poll(&mut context())
    }

    fn test_balance(
        modes: &[u8],
    ) -> (
        Balance<Backend<&'static str>, TestService, RoundRobin>,
        Vec<TestService>,
    ) {
        let services: Vec<_> = modes
            .iter()
            .copied()
            .enumerate()
            .map(|(index, mode)| TestService::new(index, mode))
            .collect();
        let endpoints = services
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, service)| {
                let id = match index {
                    0 => "a",
                    1 => "b",
                    _ => "c",
                };
                Endpoint::new(Backend::new(id), service)
            })
            .collect();
        (Balance::new(endpoints, RoundRobin::new()), services)
    }

    #[test]
    fn readiness_is_retained_and_consumed_per_selected_service() {
        let (mut balance, services) = test_balance(&[READY, READY]);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(balance.endpoints()[0].readiness(), Readiness::Ready);
        assert_eq!(balance.endpoints()[1].readiness(), Readiness::Ready);

        let mut first = Box::pin(balance.call(10));
        assert_eq!(poll_future(&mut first), Poll::Ready(Ok((0, 10))));
        assert_eq!(balance.endpoints()[0].readiness(), Readiness::Idle);
        assert_eq!(balance.endpoints()[1].readiness(), Readiness::Ready);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let mut second = Box::pin(balance.call(20));
        assert_eq!(poll_future(&mut second), Poll::Ready(Ok((1, 20))));
        assert_eq!(services[0].polls.load(Ordering::Relaxed), 2);
        assert_eq!(services[1].polls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pending_services_are_excluded_from_selection() {
        let (mut balance, services) = test_balance(&[PENDING, READY]);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let mut response = Box::pin(balance.call(7));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((1, 7))));
        assert_eq!(services[0].calls.load(Ordering::Relaxed), 0);
        assert_eq!(services[1].calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn one_readiness_failure_does_not_fail_a_healthy_pool() {
        let (mut balance, _) = test_balance(&[FAILED, READY]);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(balance.endpoints()[0].readiness(), Readiness::Failed);
        let mut response = Box::pin(balance.call(1));
        assert_eq!(poll_future(&mut response), Poll::Ready(Ok((1, 1))));
    }

    #[test]
    fn isolated_readiness_errors_reach_the_observer() {
        let (balance, _) = test_balance(&[FAILED, READY]);
        let observed = Arc::new(AtomicUsize::new(0));
        let observer_count = Arc::clone(&observed);
        let mut balance = balance.with_readiness_observer(move |index, error: &TestError| {
            assert_eq!(index, 0);
            assert_eq!(*error, TestError::Readiness);
            observer_count.fetch_add(1, Ordering::Relaxed);
        });

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(observed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_exhausting_readiness_error_preserves_endpoint_context() {
        let (mut balance, _) = test_balance(&[FAILED]);

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Err(BalanceError::Endpoint {
                index: 0,
                source: TestError::Readiness
            }))
        ));
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Err(BalanceError::NoReadyEndpoints))
        ));
    }

    #[test]
    fn failed_readiness_can_be_explicitly_retried() {
        let (mut balance, services) = test_balance(&[FAILED]);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Err(BalanceError::Endpoint { .. }))
        ));

        services[0].mode.store(READY, Ordering::Relaxed);
        balance.endpoints_mut()[0].reset_readiness();
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
    }

    #[test]
    fn ineligible_services_are_not_polled() {
        let service = TestService::new(0, READY);
        let endpoint = Endpoint::new(
            Backend::new("a").with_status(poise_core::Status::Unavailable),
            service.clone(),
        );
        let mut balance = Balance::new(vec![endpoint], RoundRobin::new());

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Err(BalanceError::NoEligibleEndpoints))
        ));
        assert_eq!(service.polls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn empty_pool_reports_a_distinct_readiness_error() {
        let mut balance: Balance<Backend<&str>, TestService, RoundRobin> =
            Balance::new(Vec::new(), RoundRobin::new());

        assert!(matches!(
            Service::<u64>::poll_ready(&mut balance, &mut context()),
            Poll::Ready(Err(BalanceError::NoEndpoints))
        ));
    }

    #[test]
    fn call_without_a_readiness_reservation_returns_selection_error() {
        let (mut balance, _) = test_balance(&[READY]);
        let mut response = Box::pin(balance.call(1));

        assert!(matches!(
            poll_future(&mut response),
            Poll::Ready(Err(BalanceError::Selection(
                PickError::NoEligibleCandidates
            )))
        ));
    }

    #[test]
    fn response_errors_preserve_endpoint_index_and_complete_load() {
        let (mut balance, services) = test_balance(&[READY]);
        services[0].response_error.store(true, Ordering::Relaxed);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let load = balance.endpoints()[0].load_tracker().clone();

        let mut response = Box::pin(balance.call(1));
        assert_eq!(load.current(), 1);
        assert!(matches!(
            poll_future(&mut response),
            Poll::Ready(Err(BalanceError::Endpoint {
                index: 0,
                source: TestError::Response
            }))
        ));
        assert_eq!(load.current(), 0);
    }

    #[test]
    fn dropping_a_response_future_releases_load_as_cancellation() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let endpoint = Endpoint::new(
            Backend::new("pending"),
            PendingService {
                dropped: Arc::clone(&dropped),
            },
        );
        let load = endpoint.load_tracker().clone();
        let mut balance = Balance::new(vec![endpoint], RoundRobin::new());
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        let response = balance.call(());
        assert_eq!(load.current(), 1);
        drop(response);
        assert_eq!(load.current(), 0);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn load_guard_distinguishes_completion_from_cancellation() {
        let completed = RecordingTracker::default();
        let endpoint = Endpoint::with_tracker(
            Backend::new("completed"),
            TestService::new(0, READY),
            completed.clone(),
        );
        let mut balance = Balance::from_parts(vec![endpoint], RoundRobin::new(), NoContext);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let mut response = Box::pin(balance.call(1));
        assert!(matches!(poll_future(&mut response), Poll::Ready(Ok(_))));
        assert_eq!(completed.state.completed.load(Ordering::Relaxed), 1);
        assert_eq!(completed.state.cancelled.load(Ordering::Relaxed), 0);

        let cancelled = RecordingTracker::default();
        let endpoint = Endpoint::with_tracker(
            Backend::new("cancelled"),
            PendingService {
                dropped: Arc::new(AtomicUsize::new(0)),
            },
            cancelled.clone(),
        );
        let mut balance = Balance::from_parts(vec![endpoint], RoundRobin::new(), NoContext);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        drop(balance.call(()));
        assert_eq!(cancelled.state.completed.load(Ordering::Relaxed), 0);
        assert_eq!(cancelled.state.cancelled.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn least_loaded_routes_around_an_outstanding_response() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let endpoints = vec![
            Endpoint::new(
                Backend::new("a"),
                PendingService {
                    dropped: Arc::clone(&dropped),
                },
            ),
            Endpoint::new(
                Backend::new("b"),
                PendingService {
                    dropped: Arc::clone(&dropped),
                },
            ),
        ];
        let loads: Vec<_> = endpoints
            .iter()
            .map(|endpoint| endpoint.load_tracker().clone())
            .collect();
        let mut balance = Balance::new(endpoints, LeastLoaded::new());

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let first = balance.call(());
        assert_eq!((loads[0].current(), loads[1].current()), (1, 0));

        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));
        let second = balance.call(());
        assert_eq!((loads[0].current(), loads[1].current()), (1, 1));

        drop((first, second));
        assert_eq!((loads[0].current(), loads[1].current()), (0, 0));
    }

    #[test]
    fn tracker_capacity_error_does_not_consume_service_readiness() {
        let service = TestService::new(0, READY);
        let load = InFlight::with_limit(NonZeroU64::MIN);
        let held = load.start().unwrap();
        let endpoint = Endpoint::with_tracker(Backend::new("a"), service, load.clone());
        let mut balance = Balance::from_parts(vec![endpoint], RoundRobin::new(), NoContext);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        let mut response = Box::pin(balance.call(1));
        assert!(matches!(
            poll_future(&mut response),
            Poll::Ready(Err(BalanceError::AtCapacity { index: 0, .. }))
        ));
        assert_eq!(balance.endpoints()[0].readiness(), Readiness::Ready);
        drop(held);
    }

    struct InvalidPolicy;

    impl<C: Candidate> Policy<C> for InvalidPolicy {
        fn pick(
            &mut self,
            _candidates: &[C],
            _context: &(),
        ) -> Result<poise_core::Selection, PickError> {
            let candidates = [Backend::new("zero"), Backend::new("one")];
            RoundRobin::with_cursor(1).pick(&candidates, &())
        }
    }

    #[test]
    fn invalid_custom_policy_indices_do_not_panic_dispatch() {
        let service = TestService::new(0, READY);
        let endpoint = Endpoint::new(Backend::new("a"), service);
        let mut balance = Balance::new(vec![endpoint], InvalidPolicy);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        let mut response = Box::pin(balance.call(1));
        assert!(matches!(
            poll_future(&mut response),
            Poll::Ready(Err(BalanceError::InvalidSelection { index: 1, len: 1 }))
        ));
    }

    #[test]
    fn request_context_supports_affinity_policies() {
        let services = [TestService::new(0, READY), TestService::new(1, READY)];
        let endpoints = vec![
            Endpoint::new(Backend::new("a"), services[0].clone()),
            Endpoint::new(Backend::new("b"), services[1].clone()),
        ];
        let mut balance =
            Balance::new(endpoints, poise_core::policy::Rendezvous::new()).with_context(UseRequest);

        let mut selected = Vec::new();
        for _ in 0..2 {
            assert!(matches!(
                balance.poll_ready(&mut context()),
                Poll::Ready(Ok(()))
            ));
            let mut response = Box::pin(balance.call(42));
            let Poll::Ready(Ok((index, _))) = poll_future(&mut response) else {
                panic!("affinity response did not complete successfully");
            };
            selected.push(index);
        }
        assert_eq!(selected[0], selected[1]);
    }

    #[test]
    fn mutable_service_access_invalidates_readiness() {
        let (mut balance, _) = test_balance(&[READY]);
        assert!(matches!(
            balance.poll_ready(&mut context()),
            Poll::Ready(Ok(()))
        ));

        let _service = balance.endpoints_mut()[0].service_mut();
        assert_eq!(balance.endpoints()[0].readiness(), Readiness::Idle);
    }
}
