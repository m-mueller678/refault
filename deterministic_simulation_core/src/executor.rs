use crate::context::{Context, CONTEXT};
use crate::event::record_event;
use crate::node::{current_node, NodeAwareFuture};
use crate::time::TimeScheduler;
use std::fmt::Display;
use std::future::Future;
use std::mem::transmute;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll::{Pending, Ready};
use std::task::{Poll, Wake, Waker};
use std::thread;

pub(crate) struct Executor {
    queue: Mutex<Vec<Arc<Task>>>,
    pub(crate) time_scheduler: Mutex<TimeScheduler>,
}

impl Executor {
    pub(crate) fn new(fast_forward_time: bool) -> Executor {
        let time_scheduler: Mutex<TimeScheduler> =
            Mutex::new(TimeScheduler::new(fast_forward_time));
        Self {
            queue: Mutex::new(vec![]),
            time_scheduler,
        }
    }

    pub(crate) fn queue(&self, task: Arc<Task>) {
        self.queue.lock().unwrap().push(task);
    }

    pub(crate) fn run(&self) {
        // Allow &self to be used within the thread that is being spawned. This operation is safe
        // since the thread is immediately being joined after it is created. Thereby, &self is never
        // used outside the run function.
        let static_selfref: &'static Self = unsafe { transmute(self) };

        // Run the simulation on a new thread to avoid thread local state on this thread interfering
        // with random number generation
        thread::spawn(move || {
            while let Some(task) = static_selfref.next_queue_item() {
                task.poll();
                record_event(FuturePolledEvent::new());

                if static_selfref.queue.lock().unwrap().is_empty() {
                    static_selfref
                        .time_scheduler
                        .lock()
                        .unwrap()
                        .wait_until_next_future_ready();
                }
            }
        })
        .join()
        .unwrap();
    }

    fn next_queue_item(&self) -> Option<Arc<Task>> {
        let mut queue = self.queue.lock().unwrap();
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }
}

pub(crate) struct Task {
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>>,
}

impl Task {
    pub fn new(future: impl Future<Output = ()> + Send + Sync + 'static) -> Task {
        Self {
            future: Mutex::new(Box::pin(future)),
        }
    }

    fn poll(self: Arc<Self>) {
        let waker = Waker::from(Arc::new(NotifyingWaker::new(self.clone())));
        let _ = self
            .future
            .lock()
            .unwrap()
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker));
    }
}

struct NotifyingWaker {
    task: Arc<Task>,
}

impl NotifyingWaker {
    fn new(task: Arc<Task>) -> Self {
        Self { task }
    }
}

impl Wake for NotifyingWaker {
    fn wake(self: Arc<Self>) {
        let mut context = CONTEXT.lock().unwrap();
        let context: &mut Context = context.as_mut().unwrap();
        context.executor.queue(self.task.clone());
    }
}

pub fn spawn<T: Send + 'static>(
    future: impl Future<Output = T> + Send + Sync + 'static,
) -> TaskTrackingFuture<T> {
    record_event(TaskSpawnedEvent::new());

    let state = Arc::new(Mutex::new(TaskTrackingFutureState::new()));
    let observing_future = ObservingFuture {
        state: state.clone(),
        inner: Mutex::new(Box::pin(future)),
    };

    let node_option = current_node();
    let task = Arc::new(match node_option {
        Some(node) => Task::new(NodeAwareFuture {
            id: node,
            inner: Mutex::new(Box::pin(observing_future)),
        }),
        None => Task::new(observing_future),
    });
    let mut context = CONTEXT.lock().unwrap();
    let context: &mut Context = context.as_mut().unwrap();
    context.executor.queue(task);

    TaskTrackingFuture { inner: state }
}

pub struct TaskTrackingFuture<T> {
    pub(crate) inner: Arc<Mutex<TaskTrackingFutureState<T>>>,
}

impl<T> TaskTrackingFuture<T> {
    pub fn query_result(&self) -> Option<T> {
        self.inner.lock().unwrap().result.take()
    }
}

pub(crate) struct TaskTrackingFutureState<T> {
    result: Option<T>,
    waker: Option<Waker>,
}

impl<T> TaskTrackingFutureState<T> {
    pub(crate) fn new() -> Self {
        Self {
            result: None,
            waker: None,
        }
    }
}

impl<T> Future for TaskTrackingFuture<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let mut state = self.inner.lock().unwrap();
        match state.result.take() {
            Some(value) => Ready(value),
            None => {
                state.waker = Some(cx.waker().clone());
                Pending
            }
        }
    }
}

pub(crate) struct ObservingFuture<T> {
    pub(crate) state: Arc<Mutex<TaskTrackingFutureState<T>>>,
    pub(crate) inner: Mutex<Pin<Box<dyn Future<Output = T> + Send + Sync + 'static>>>,
}

impl<T> Future for ObservingFuture<T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<()> {
        let context = &mut std::task::Context::from_waker(&cx.waker());
        let poll = self.inner.lock().unwrap().as_mut().poll(context);

        match poll {
            Ready(result) => {
                let waker_option = {
                    let mut state = self.state.lock().unwrap();
                    state.result = Some(result);
                    state.waker.take()
                };

                match waker_option {
                    Some(waker) => {
                        waker.wake();
                    }
                    _ => {}
                }
                Ready(())
            }
            _ => Pending,
        }
    }
}

struct TaskSpawnedEvent {}

impl TaskSpawnedEvent {
    fn new() -> Box<Self> {
        Box::new(Self {})
    }
}

impl Display for TaskSpawnedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from("TaskSpawnedEvent{}"))
    }
}

struct FuturePolledEvent {}

impl FuturePolledEvent {
    fn new() -> Box<Self> {
        Box::new(Self {})
    }
}

impl Display for FuturePolledEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from("FuturePolledEvent{}"))
    }
}
