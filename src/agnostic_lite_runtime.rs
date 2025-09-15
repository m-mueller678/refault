mod timeout;

use crate::{
    SimCx,
    executor::{TaskAborted, spawn},
    send_bind::{SimBound, SimNodeBound},
    time::{Sleep, sleep, sleep_until},
};
use agnostic_lite::{
    AfterHandle, AsyncAfterSpawner, AsyncBlockingSpawner, AsyncLocalSpawner, AsyncSpawner,
    JoinHandle, LocalJoinHandle, RuntimeLite, Yielder,
    time::{AsyncLocalInterval, AsyncLocalSleep, AsyncLocalTimeout, AsyncTimeout, Delay},
};
use futures::{FutureExt, Stream};
use std::{future::ready, pin::Pin, time::Instant};
use timeout::Timeout;

#[derive(Clone, Copy)]
pub struct SimRuntime;

impl RuntimeLite for SimRuntime {
    type Spawner = Spawner;
    type LocalSpawner = Spawner;
    type BlockingSpawner = Spawner;
    type Instant = std::time::Instant;
    type AfterSpawner = Spawner;
    type Interval = Interval;
    type LocalInterval = Interval;
    type Sleep = SimNodeBound<Sleep>;
    type LocalSleep = SimNodeBound<Sleep>;
    type Delay<F: Future + Send> = Delay<F, Self::Sleep>;
    type LocalDelay<F: Future> = Delay<F, Self::LocalSleep>;
    type Timeout<F: Future + Send> = Timeout<F>;
    type LocalTimeout<F: Future> = Timeout<F>;

    fn new() -> Self {
        SimCx::with(|cx| {
            cx.cxu();
        });
        SimRuntime
    }

    fn name() -> &'static str {
        Self::fqname().rsplit(':').next().unwrap()
    }

    fn fqname() -> &'static str {
        std::any::type_name::<Self>()
    }

    fn block_on<F: Future>(_f: F) -> F::Output {
        unimplemented!()
    }

    fn yield_now() -> impl Future<Output = ()> + Send {
        Self::sleep_until(Instant::now()).map(drop)
    }

    fn interval(_interval: core::time::Duration) -> Self::Interval {
        todo!()
    }

    fn interval_at(_start: Self::Instant, _period: core::time::Duration) -> Self::Interval {
        todo!()
    }

    fn interval_local(interval: core::time::Duration) -> Self::LocalInterval {
        Self::interval(interval)
    }

    fn interval_local_at(
        start: Self::Instant,
        period: core::time::Duration,
    ) -> Self::LocalInterval {
        Self::interval_at(start, period)
    }

    fn sleep(duration: core::time::Duration) -> Self::Sleep {
        sleep(duration).into()
    }

    fn sleep_until(instant: Self::Instant) -> Self::Sleep {
        sleep_until(instant).into()
    }

    fn sleep_local(duration: core::time::Duration) -> Self::LocalSleep {
        Self::sleep(duration)
    }

    fn sleep_local_until(instant: Self::Instant) -> Self::LocalSleep {
        Self::sleep_until(instant)
    }

    fn delay<F>(duration: core::time::Duration, fut: F) -> Self::Delay<F>
    where
        F: Future + Send,
    {
        Self::delay_local(duration, fut)
    }

    fn delay_local<F>(_duration: core::time::Duration, _fut: F) -> Self::LocalDelay<F>
    where
        F: Future,
    {
        todo!()
    }

    fn delay_at<F>(deadline: Self::Instant, fut: F) -> Self::Delay<F>
    where
        F: Future + Send,
    {
        Self::delay_local_at(deadline, fut)
    }

    fn delay_local_at<F>(_deadline: Self::Instant, _fut: F) -> Self::LocalDelay<F>
    where
        F: Future,
    {
        todo!()
    }

    fn timeout<F>(duration: core::time::Duration, future: F) -> Self::Timeout<F>
    where
        F: Future + Send,
    {
        Self::Timeout::timeout(duration, future)
    }

    fn timeout_at<F>(deadline: Self::Instant, future: F) -> Self::Timeout<F>
    where
        F: Future + Send,
    {
        Self::Timeout::timeout_at(deadline, future)
    }

    fn timeout_local<F>(duration: core::time::Duration, future: F) -> Self::LocalTimeout<F>
    where
        F: Future,
    {
        Self::LocalTimeout::timeout_local(duration, future)
    }

    fn timeout_local_at<F>(deadline: Self::Instant, future: F) -> Self::LocalTimeout<F>
    where
        F: Future,
    {
        Self::LocalTimeout::timeout_local_at(deadline, future)
    }
}

#[derive(Clone, Copy)]
pub struct Spawner {}

impl<O> JoinHandle<O> for SimBound<crate::executor::TaskHandle<O>> {
    type JoinError = TaskAborted;

    fn abort(self) {
        self.unwrap_sim_bound().abort();
    }

    fn detach(self)
    where
        Self: Sized,
    {
        self.unwrap_sim_bound().detach();
    }
}

impl<O> LocalJoinHandle<O> for SimBound<crate::executor::TaskHandle<O>> {
    type JoinError = TaskAborted;

    fn detach(self)
    where
        Self: Sized,
    {
        self.unwrap_sim_bound().detach();
    }
}

impl<O: Send + 'static> AfterHandle<O> for SimBound<crate::executor::TaskHandle<O>> {
    type JoinError = TaskAborted;

    fn cancel(self) -> impl Future<Output = Option<Result<O, Self::JoinError>>> + Send {
        #[allow(unreachable_code)]
        ready(todo!())
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

impl Yielder for Spawner {
    fn yield_now() -> impl Future<Output = ()> + Send {
        SimRuntime::yield_now()
    }

    fn yield_now_local() -> impl Future<Output = ()> {
        SimRuntime::yield_now()
    }
}

impl AsyncSpawner for Spawner {
    type JoinHandle<O: Send + 'static> = SimBound<crate::executor::TaskHandle<O>>;

    fn spawn<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F::Output: Send + 'static,
        F: Future + Send + 'static,
    {
        spawn(future).into()
    }
}

impl AsyncLocalSpawner for Spawner {
    type JoinHandle<O: 'static> = SimBound<crate::executor::TaskHandle<O>>;

    fn spawn_local<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F::Output: 'static,
        F: Future + 'static,
    {
        spawn(future).into()
    }
}

impl AsyncBlockingSpawner for Spawner {
    type JoinHandle<R: Send + 'static> = SimBound<crate::executor::TaskHandle<R>>;

    fn spawn_blocking<F, R>(_f: F) -> Self::JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        todo!()
    }
}

impl AsyncAfterSpawner for Spawner {
    type Instant = Instant;

    type JoinHandle<F: Send + 'static> = SimBound<crate::executor::TaskHandle<F>>;

    fn spawn_after<F>(duration: std::time::Duration, future: F) -> Self::JoinHandle<F::Output>
    where
        F::Output: Send + 'static,
        F: Future + Send + 'static,
    {
        SimRuntime::spawn_after(duration, future)
    }

    fn spawn_after_at<F>(instant: Self::Instant, future: F) -> Self::JoinHandle<F::Output>
    where
        F::Output: Send + 'static,
        F: Future + Send + 'static,
    {
        SimRuntime::spawn_after_at(instant, future)
    }
}

pub struct Interval {}

impl AsyncLocalInterval for Interval {
    type Instant = Instant;

    fn reset(&mut self, _interval: std::time::Duration) {
        todo!()
    }

    fn reset_at(&mut self, _instant: Self::Instant) {
        todo!()
    }

    fn poll_tick(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Instant> {
        todo!()
    }
}

impl Stream for Interval {
    type Item = Instant;

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        todo!()
    }
}

impl AsyncLocalSleep for SimNodeBound<Sleep> {
    type Instant = Instant;

    fn reset(mut self: Pin<&mut Self>, deadline: Self::Instant) {
        self.set(sleep_until(deadline).into())
    }
}
