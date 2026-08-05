use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{BalanceError, LoadGuard};

pin_project! {
    /// Response future returned by [`Balance`](crate::Balance).
    ///
    /// A pending future owns its endpoint load guard. Normal service completion,
    /// including an endpoint error, explicitly completes the guard. Dropping the
    /// future first treats the attempt as cancellation.
    pub struct ResponseFuture<F, G, E> {
        #[pin]
        future: Option<F>,
        guard: Option<G>,
        endpoint: usize,
        error: Option<BalanceError<E>>,
        completed: bool,
    }
}

impl<F, G, E> ResponseFuture<F, G, E> {
    pub(crate) fn running(future: F, guard: G, endpoint: usize) -> Self {
        Self {
            future: Some(future),
            guard: Some(guard),
            endpoint,
            error: None,
            completed: false,
        }
    }

    pub(crate) fn failed(error: BalanceError<E>) -> Self {
        Self {
            future: None,
            guard: None,
            endpoint: 0,
            error: Some(error),
            completed: false,
        }
    }
}

impl<F, G, Response, E> Future for ResponseFuture<F, G, E>
where
    F: Future<Output = Result<Response, E>>,
    G: LoadGuard,
{
    type Output = Result<Response, BalanceError<E>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        assert!(
            !*this.completed,
            "a completed Poise response future was polled again"
        );

        if let Some(error) = this.error.take() {
            *this.completed = true;
            return Poll::Ready(Err(error));
        }

        let Some(future) = this.future.as_mut().as_pin_mut() else {
            panic!("a Poise response future has neither a service future nor an error");
        };
        let result = match future.poll(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        this.future.set(None);
        this.guard
            .take()
            .expect("a running Poise response future must own a load guard")
            .complete();
        *this.completed = true;

        Poll::Ready(result.map_err(|source| BalanceError::Endpoint {
            index: *this.endpoint,
            source,
        }))
    }
}
