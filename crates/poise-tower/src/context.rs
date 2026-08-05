/// Derives policy context from an incoming request.
///
/// Implementations borrow the request rather than allocate an owned routing
/// key. [`NoContext`] supports request-independent policies, while
/// [`UseRequest`] passes the complete request to keyed policies.
pub trait RequestContext<Request> {
    /// The context type consumed by the policy.
    type Context: ?Sized;

    /// Borrows policy context for this request.
    fn context<'a>(&'a self, request: &'a Request) -> &'a Self::Context;
}

/// Supplies unit context to a request-independent policy.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct NoContext;

impl<Request> RequestContext<Request> for NoContext {
    type Context = ();

    fn context<'a>(&'a self, _request: &'a Request) -> &'a Self::Context {
        &()
    }
}

/// Uses the complete request as policy context.
///
/// This is useful with affinity policies when the request itself is a stable
/// hash key. Applications can implement [`RequestContext`] on their own type to
/// project a field instead.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct UseRequest;

impl<Request> RequestContext<Request> for UseRequest {
    type Context = Request;

    fn context<'a>(&'a self, request: &'a Request) -> &'a Self::Context {
        request
    }
}
