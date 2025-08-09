use crate::event::Event;
use crate::{
    context::{Context2, NodeId, with_context},
    time::TimeScheduler,
};
use cooked_waker::{IntoWaker, WakeRef};
use futures_channel::oneshot;
use std::mem;
use std::sync::atomic::Ordering::Relaxed;
use std::task::Poll;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, atomic::AtomicUsize},
    task::Context,
};

static EXECUTOR_ID: AtomicUsize = AtomicUsize::new(0);

pub struct ExecutorQueue {
    ready_queue: VecDeque<usize>,
    id: usize,
}

impl ExecutorQueue {
    pub fn is_empty(&self) -> bool {
        self.ready_queue.is_empty()
    }

    pub fn new() -> Self {
        ExecutorQueue {
            ready_queue: VecDeque::new(),
            id: EXECUTOR_ID.fetch_add(1, Relaxed),
        }
    }

    pub fn executor(&self) -> Executor {
        Executor {
            id: self.id,
            tasks: HashMap::new(),
            next_task_id: 0,
            time_scheduler: TimeScheduler::new(),
        }
    }
}

pub struct Executor {
    id: usize,
    // each entry corresponds to one TaskShared
    // entries are None while the task is executing, cancelled, or completed
    #[allow(clippy::type_complexity)]
    tasks: HashMap<usize, Cell<Option<Pin<Box<dyn TaskDyn>>>>>,
    next_task_id: usize,
    pub time_scheduler: TimeScheduler,
}

pin_project_lite::pin_project! {
    struct Task<F:Future>{
        shared:Arc<TaskShared>,
        snd:Option<oneshot::Sender<F::Output>>,
        #[pin]
        fut: F,
    }
}

trait TaskDyn {
    fn run(self: Pin<&mut Self>) -> bool;
    fn as_base(&self) -> &Arc<TaskShared>;
}

impl<F: Future> TaskDyn for Task<F> {
    fn run(self: Pin<&mut Self>) -> bool {
        let this = self.project();
        match this.shared.state.load(Relaxed) {
            TASK_CANCELLED => false,
            TASK_READY => {
                this.shared.state.store(TASK_WAITING_OR_COMPLETE, Relaxed);
                let waker = this.shared.clone().into_waker();
                match this.fut.poll(&mut Context::from_waker(&waker)) {
                    Poll::Ready(x) => {
                        this.snd.take().unwrap().send(x).ok();
                        false
                    }
                    Poll::Pending => true,
                }
            }
            TASK_WAITING_OR_COMPLETE | TASK_END.. => {
                unreachable!()
            }
        }
    }

    fn as_base(&self) -> &Arc<TaskShared> {
        &self.shared
    }
}

pin_project_lite::pin_project! {
    pub struct TaskHandle<T> {
        #[pin]
        result: oneshot::Receiver<T>,
        abort: AbortHandle,
    }
}

pub struct AbortHandle(Arc<TaskShared>);

const TASK_CANCELLED: usize = 0;
const TASK_READY: usize = 1;
const TASK_WAITING_OR_COMPLETE: usize = 2;
const TASK_END: usize = 3;

struct TaskShared {
    state: AtomicUsize,
    executor_id: usize,
    id: usize,
    node: NodeId,
}

impl WakeRef for TaskShared {
    fn wake_by_ref(&self) {
        Context2::with(|cx| {
            let mut ex = cx.queue.borrow_mut();
            let ex = ex.as_mut().unwrap();
            assert_eq!(ex.id, self.executor_id);
            match self.state.load(Relaxed) {
                TASK_CANCELLED | TASK_READY => (),
                TASK_WAITING_OR_COMPLETE => {
                    self.state.store(TASK_READY, Relaxed);
                    ex.ready_queue.push_back(self.id);
                }
                TASK_END.. => unreachable!(),
            };
        });
    }
}

impl Drop for TaskShared {
    fn drop(&mut self) {
        with_context(|cx| {
            assert_eq!(cx.executor.id, self.executor_id);
            cx.executor.tasks.remove(&self.id);
        })
    }
}

impl Executor {
    pub fn spawn<F: Future + 'static>(&mut self, node: NodeId, future: F) -> TaskHandle<F::Output> {
        let (snd, rcv) = oneshot::channel();
        let task_id = self.next_task_id;
        self.next_task_id = self.next_task_id.checked_add(1).unwrap();
        let task = Box::pin(Task {
            snd: Some(snd),
            fut: future,
            shared: Arc::new(TaskShared {
                state: AtomicUsize::new(TASK_WAITING_OR_COMPLETE),
                id: task_id,
                node,
                executor_id: self.id,
            }),
        });
        let task_handle = TaskHandle {
            result: rcv,
            abort: AbortHandle(task.shared.clone()),
        };
        self.tasks.insert(task_id, Cell::new(Some(task)));
        <TaskShared as WakeRef>::wake_by_ref(&*task_handle.abort.0);
        task_handle
    }

    pub fn run_current_context() {
        loop {
            while let Some(mut task) = Context2::with(|cx| {
                let task_id = cx
                    .queue
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .ready_queue
                    .pop_front()?;
                let mut cx = cx.context.borrow_mut();
                let cx = cx.as_mut().unwrap();
                let task = cx.executor.tasks.get_mut(&task_id).unwrap().take()?;
                cx.event_handler
                    .handle_event(Event::TaskRun(task.as_base().id));
                assert!(cx.current_node == NodeId::INIT);
                cx.current_node = task.as_base().node;
                Some(task)
            }) {
                let keep = task.as_mut().run();
                with_context(|cx| {
                    debug_assert_eq!(cx.current_node, task.as_base().node);
                    cx.current_node = NodeId::INIT;
                    if keep {
                        assert!(
                            cx.executor
                                .tasks
                                .get_mut(&task.as_base().id)
                                .unwrap()
                                .replace(Some(task))
                                .is_none()
                        );
                        None
                    } else {
                        // return the task outside the context scope, as drop may attempt to lock it.
                        Some(task)
                    }
                });
            }
            if !Context2::with(|cx2| {
                let mut cx = cx2.context.borrow_mut();
                let cx = cx.as_mut().unwrap();
                cx.executor
                    .time_scheduler
                    .wait_until_next_future_ready(&cx2.time, &mut *cx.event_handler)
            }) {
                break;
            }
        }
        let tasks = with_context(|cx| mem::take(&mut cx.executor.tasks));
        for task in tasks {
            if cfg!(debug_assertions)
                && let Some(task) = task.1.into_inner()
            {
                match task.as_base().state.load(Relaxed) {
                    TASK_CANCELLED => (),
                    TASK_READY | TASK_END.. => unreachable!(),
                    TASK_WAITING_OR_COMPLETE => {
                        eprintln!("task still waiting");
                    }
                }
            }
        }
    }
}

impl AbortHandle {
    pub fn abort(self) {
        self.0.state.store(TASK_CANCELLED, Relaxed);
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().result.poll(cx).map(Result::ok)
    }
}

/// Spawn a task on the current node
pub fn spawn<F: Future + 'static>(future: F) -> TaskHandle<F::Output> {
    with_context(|cx| {
        cx.event_handler.handle_event(Event::TaskSpawned);
        cx.executor.spawn(cx.current_node, future)
    })
}

/// Spawn a task on the specified node.
pub fn spawn_on_node<F: Future + 'static>(node: NodeId, future: F) -> TaskHandle<F::Output> {
    with_context(|cx| {
        cx.event_handler.handle_event(Event::TaskSpawned);
        cx.executor.spawn(node, future)
    })
}
