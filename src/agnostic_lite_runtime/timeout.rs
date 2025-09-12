use std::{task::Poll, time::Instant};

use agnostic_lite::time::Elapsed;

use crate::{
    send_bind::SimNodeBound,
    time::{Sleep, sleep_until},
};

pin_project_lite::pin_project! {
    pub struct Timeout<F> {
        #[pin]
        fut: F,
        #[pin]
        sleep: SimNodeBound<Sleep>,
    }
}

impl<F: Future + Send> agnostic_lite::time::AsyncTimeout<F> for Timeout<F> {
    type Instant = Instant;

    fn timeout(timeout: std::time::Duration, fut: F) -> Self
    where
        F: Future + Send,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Send + Sized,
    {
        Self::timeout_at(Instant::now() + timeout, fut)
    }

    fn timeout_at(deadline: Self::Instant, fut: F) -> Self
    where
        F: Future + Send,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Send + Sized,
    {
        Timeout {
            fut,
            sleep: sleep_until(deadline).into(),
        }
    }
}

impl<F: Future> agnostic_lite::time::AsyncLocalTimeout<F> for Timeout<F> {
    type Instant = Instant;

    fn timeout_local(timeout: std::time::Duration, fut: F) -> Self
    where
        F: Future,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Sized,
    {
        Self::timeout_local_at(Instant::now() + timeout, fut)
    }

    fn timeout_local_at(deadline: Self::Instant, fut: F) -> Self
    where
        F: Future,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Sized,
    {
        Timeout {
            fut,
            sleep: sleep_until(deadline).into(),
        }
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.sleep.poll(cx) {
            Poll::Ready(_) => return Poll::Ready(Err(Elapsed)),
            Poll::Pending => (),
        }
        match this.fut.poll(cx) {
            Poll::Ready(x) => Poll::Ready(Ok(x)),
            Poll::Pending => Poll::Pending,
        }
    }
}
