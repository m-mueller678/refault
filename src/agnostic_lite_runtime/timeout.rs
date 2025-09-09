use std::{task::Poll, thread::sleep_until, time::Instant};

use agnostic_lite::time::Elapsed;

use crate::time::Sleep;

pin_project_lite::pin_project! {
    pub struct Timeout<F> {
        #[pin]
        fut: F,
        #[pin]
        sleep: Sleep,
    }
}

impl<F> agnostic_lite::time::AsyncTimeout for Timeout<F> {
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
            sleep: sleep_until(deadline),
        }
    }
}

impl<F> agnostic_lite::time::AsyncLocalTimeout for Timeout<F> {
    type Instant = Instant;

    fn timeout_local(timeout: std::time::Duration, fut: F) -> Self
    where
        F: Future + Send,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Send + Sized,
    {
        Self::timeout_at(Instant::now() + timeout, fut)
    }

    fn timeout_local_at(deadline: Self::Instant, fut: F) -> Self
    where
        F: Future + Send,
        Self: Future<Output = Result<F::Output, agnostic_lite::time::Elapsed>> + Send + Sized,
    {
        Timeout {
            fut,
            sleep: sleep_until(deadline),
        }
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.sleep.poll(cx) {
            Poll::Ready => return Err(Elapsed),
            Poll::Pending => (),
        }
        match this.fut.poll(cx) {
            Poll::Ready(x) => Ok(x),
            Poll::Pending => Poll::Pending,
        }
    }
}
