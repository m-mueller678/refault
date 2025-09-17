use crate::{
    executor::{TaskAborted, TaskHandle},
    send_bind::SimNodeBound,
};
use agnostic_lite::{AfterHandle, LocalJoinHandle};
use futures::FutureExt;
use std::{future::ready, pin::Pin};

pub struct JoinHandle<T>(SimNodeBound<TaskHandle<T>>);

impl<T> JoinHandle<T> {
    pub fn into_inner(self) -> SimNodeBound<TaskHandle<T>> {
        self.0
    }
}

impl<T> From<SimNodeBound<TaskHandle<T>>> for JoinHandle<T> {
    fn from(value: SimNodeBound<TaskHandle<T>>) -> Self {
        JoinHandle(value)
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, TaskAborted>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.get_mut().0.poll_unpin(cx)
    }
}

impl<T> agnostic_lite::JoinHandle<T> for JoinHandle<T> {
    type JoinError = TaskAborted;

    fn abort(self) {
        self.0.unwrap_sim_bound().unwrap_node_bound().abort();
    }

    fn detach(self)
    where
        Self: Sized,
    {
        self.0.unwrap_sim_bound().unwrap_node_bound().detach();
    }
}

impl<T> LocalJoinHandle<T> for JoinHandle<T> {
    type JoinError = TaskAborted;

    fn detach(self)
    where
        Self: Sized,
    {
        self.0.unwrap_sim_bound().unwrap_node_bound().detach();
    }
}

impl<T: Send + 'static> AfterHandle<T> for JoinHandle<T> {
    type JoinError = TaskAborted;

    fn cancel(self) -> impl Future<Output = Option<Result<T, Self::JoinError>>> + Send {
        #[allow(unreachable_code)]
        ready::<Option<Result<T, TaskAborted>>>(todo!())
    }

    fn reset(&self, _duration: core::time::Duration) {
        todo!()
    }

    fn abort(self) {
        todo!()
    }

    fn is_expired(&self) -> bool {
        todo!()
    }

    fn is_finished(&self) -> bool {
        todo!()
    }
}
