use crate::context::with_context_option;
use crate::event::Event;
use crate::simulator::for_all_simulators;
use crate::{
    context::time::TimeScheduler,
    context::{Context2, NodeId, with_context},
};
use cooked_waker::{IntoWaker, WakeRef};
use futures_channel::oneshot;
use std::collections::HashSet;
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

use super::ContextAnchor;

pub struct ExecutorQueue {
    ready_queue: VecDeque<usize>,
}

impl ExecutorQueue {
    pub fn is_empty(&self) -> bool {
        self.ready_queue.is_empty()
    }

    pub fn new() -> Self {
        ExecutorQueue {
            ready_queue: VecDeque::new(),
        }
    }

    pub fn executor(&self) -> Executor {
        Executor {
            tasks: HashMap::new(),
            tasks_by_node: HashMap::new(),
            next_task_id: 0,
            time_scheduler: TimeScheduler::new(),
        }
    }
}

pub struct Executor {
    // each entry corresponds to one TaskShared
    // entries are None while the task is executing, cancelled, or completed
    #[allow(clippy::type_complexity)]
    tasks: HashMap<usize, TaskEntry>,
    tasks_by_node: HashMap<NodeId, HashSet<usize>>,
    next_task_id: usize,
    pub time_scheduler: TimeScheduler,
}

struct TaskEntry {
    shared: Arc<TaskShared>,
    task: Cell<Option<Pin<Box<dyn TaskDyn>>>>,
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
        let waker = this.shared.clone().into_waker();
        match this.fut.poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(x) => {
                this.snd.take().unwrap().send(x).ok();
                this.shared.state.store(TASK_COMPLETE, Relaxed);
                false
            }
            Poll::Pending => true,
        }
    }

    fn as_base(&self) -> &Arc<TaskShared> {
        &self.shared
    }
}

pin_project_lite::pin_project! {
    /// A handle to a task.
    ///
    /// A task handle can be used to await a tasks completion or to abort it.
    /// Awaiting it will return the value returned by the spawned future or `None` if the task was aborted.
    ///
    /// A task handle must either be polled to completion or destroyd via [detach](Self::detach) or [abort](Self::abort).
    /// Dropping the handle without doing any of those will panic.
    pub struct TaskHandle<T> {
        #[pin]
        result: oneshot::Receiver<T>,
        abort: AbortHandle,
        droppable:bool,
    }

    impl<T> PinnedDrop for TaskHandle<T> {
        fn drop(this: Pin<&mut Self>) {
            assert!(
                this.droppable,
                "TaskHandle dropped. You must call either abort or detach it."
            )
        }
    }
}

impl<T> TaskHandle<T> {
    /// Abort the associated task.
    pub fn abort(mut self) {
        self.abort.abort_inner();
        self.droppable = true;
    }

    /// Detach this handle from the task, allowing the task to keep running.
    pub fn detach(mut self) {
        self.droppable = true;
    }

    /// Obtain a handle that can be used to abort the associated task.
    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }
}

/// A handle that can be used to abort a task.
///
/// Dropping this will not abort the task.
#[derive(Clone)]
pub struct AbortHandle(Arc<TaskShared>);

const TASK_CANCELLED: usize = 0;
const TASK_READY: usize = 1;
const TASK_WAITING: usize = 2;
const TASK_COMPLETE: usize = 3;
const TASK_END: usize = 4;

struct TaskShared {
    anchor: ContextAnchor,
    state: AtomicUsize,
    id: usize,
    node: NodeId,
}

impl WakeRef for TaskShared {
    fn wake_by_ref(&self) {
        Context2::with(|cx| {
            self.anchor.check();
            let mut ex = cx.queue.borrow_mut();
            let ex = ex.as_mut().unwrap();
            match self.state.load(Relaxed) {
                TASK_CANCELLED | TASK_COMPLETE | TASK_READY => (),
                TASK_WAITING => {
                    self.state.store(TASK_READY, Relaxed);
                    ex.ready_queue.push_back(self.id);
                }
                TASK_END.. => unreachable!(),
            };
        });
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
                state: AtomicUsize::new(TASK_WAITING),
                id: task_id,
                node,
            }),
        });
        let task_entry = TaskEntry {
            shared: task.shared.clone(),
            task: Cell::new(Some(task)),
        };
        let task_handle = TaskHandle {
            result: rcv,
            abort: AbortHandle(task_entry.shared.clone()),
            droppable: false,
        };
        self.tasks.insert(task_id, task_entry);
        self.tasks_by_node.entry(node).or_default().insert(task_id);
        <TaskShared as WakeRef>::wake_by_ref(&*task_handle.abort.0);
        task_handle
    }

    fn remove_task_entry(&mut self, task_id: usize) {
        let removed = self.tasks.remove(&task_id);
        let task_entry = removed.unwrap();
        debug_assert_eq!(task_entry.shared.id, task_id);
        assert!(task_entry.task.into_inner().is_none());
        let node = self.tasks_by_node.get_mut(&task_entry.shared.node).unwrap();
        let removed = node.remove(&task_id);
        debug_assert!(removed);
    }

    pub fn run_current_context() {
        loop {
            while let Some(mut task) = Context2::with(|cx| {
                loop {
                    let task_id = cx
                        .queue
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .ready_queue
                        .pop_front()?;
                    let mut cx = cx.context.borrow_mut();
                    let cx = cx.as_mut().unwrap();
                    let ex = &mut cx.executor;
                    let task_entry = ex.tasks.get_mut(&task_id).unwrap();
                    match task_entry.shared.state.load(Relaxed) {
                        TASK_READY => (),
                        TASK_CANCELLED => {
                            let id = task_entry.shared.id;
                            ex.remove_task_entry(id);
                            continue;
                        }
                        TASK_COMPLETE | TASK_WAITING | TASK_END.. => unreachable!(),
                    }
                    let task = task_entry.task.take().unwrap();
                    cx.event_handler
                        .handle_event(Event::TaskRun(task_entry.shared.id));
                    debug_assert!(cx.current_node == NodeId::INIT);
                    cx.current_node = task_entry.shared.node;
                    task_entry.shared.state.store(TASK_WAITING, Relaxed);
                    break Some(task);
                }
            }) {
                let keep = task.as_mut().run();
                with_context(|cx| {
                    let base = task.as_base();
                    debug_assert_eq!(cx.current_node, base.node);
                    cx.current_node = NodeId::INIT;
                    if keep {
                        let task_entry = cx.executor.tasks.get_mut(&base.id).unwrap();
                        assert!(task_entry.task.replace(Some(task)).is_none());
                    } else {
                        cx.executor.remove_task_entry(base.id);
                    }
                });
            }
            if !Context2::with(|cx2| {
                cx2.with_cx(|cx| {
                    cx.executor
                        .time_scheduler
                        .wait_until_next_future_ready(&cx2.time, &mut *cx.event_handler)
                })
            }) {
                break;
            }
        }
        let tasks = with_context(|cx| mem::take(&mut cx.executor.tasks));
        if cfg!(debug_assertions) {
            for (_, TaskEntry { shared, task }) in tasks {
                match shared.state.load(Relaxed) {
                    TASK_CANCELLED | TASK_COMPLETE => {
                        assert!(task.into_inner().is_none());
                    }
                    TASK_READY | TASK_END.. => unreachable!(),
                    TASK_WAITING => {
                        assert!(task.into_inner().is_some());
                        eprintln!("task still waiting");
                    }
                }
            }
        }
    }
}

fn abort_local(
    task_id: usize,
    queue: &mut ExecutorQueue,
    tasks: &mut HashMap<usize, TaskEntry>,
) -> Option<Pin<Box<dyn TaskDyn>>> {
    let task_entry = tasks.get_mut(&task_id).unwrap();
    match task_entry.shared.state.load(Relaxed) {
        TASK_CANCELLED | TASK_COMPLETE => None,
        TASK_WAITING | TASK_READY => {
            task_entry.shared.state.store(TASK_CANCELLED, Relaxed);
            queue.ready_queue.push_back(task_id);
            let task = task_entry.task.take().unwrap();
            Some(task)
        }
        TASK_END.. => unreachable!(),
    }
}

impl AbortHandle {
    /// Abort the associated task.
    pub fn abort(self) {
        self.abort_inner();
    }

    fn abort_inner(&self) {
        drop(Context2::with_in_node(self.0.node, |cx| {
            self.0.anchor.check();
            let mut queue_guard = cx.queue.borrow_mut();
            let queue = queue_guard.as_mut().unwrap();
            abort_local(
                self.0.id,
                queue,
                &mut cx.context.borrow_mut().as_mut().unwrap().executor.tasks,
            )
            // drop task outside all guards
        }));
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.result.poll(cx) {
            Poll::Ready(x) => {
                *this.droppable = true;
                Poll::Ready(x.ok())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Spawn a task on the current node.
pub fn spawn<F: Future + 'static>(future: F) -> TaskHandle<F::Output> {
    NodeId::current().spawn(future)
}

impl NodeId {
    /// Create a new node that tasks can be run on.
    ///
    /// Can only be called from within a simulation.
    pub fn create_node() -> NodeId {
        with_context(|cx| cx.new_node())
    }

    /// Spawn a task on this node.
    ///
    /// Can only be called from within a simulation.
    pub fn spawn<F: Future + 'static>(self, future: F) -> TaskHandle<F::Output> {
        with_context(|cx| {
            assert!(!cx.stopped[self.0]);
            cx.event_handler.handle_event(Event::TaskSpawned);
            cx.executor.spawn(self, future)
        })
    }

    /// Returns the id of the node this task is running on.
    ///
    /// Can only be called from within a simulation.
    pub fn current() -> Self {
        with_context(|cx| cx.current_node)
    }

    /// Returns the id of the current node if within the simulation.
    /// Returns `None` otherwise.
    pub fn try_current() -> Option<Self> {
        with_context_option(|cx| {
            if let Some(cx) = cx {
                Some(cx.current_node)
            } else {
                None
            }
        })
    }

    /// Invoke Simulator::stop on all simulators and stop all tasks on this node.
    ///
    /// Attempting to spawn tasks on a stopped node will panic.
    pub fn stop(self) {
        Context2::with_in_node(self, |cx2| {
            for_all_simulators(false, |x| x.stop_node());
            let task_ids = {
                cx2.with_cx(|context| {
                    let node = context.executor.tasks_by_node.get(&self);
                    node.into_iter().flatten().copied().collect::<Vec<usize>>()
                })
            };
            for &task in &task_ids {
                drop(cx2.with_cx(|cx| {
                    abort_local(
                        task,
                        cx2.queue.borrow_mut().as_mut().unwrap(),
                        &mut cx.executor.tasks,
                    )
                }))
            }
        });
    }

    /// Iterate over all nodes in the current simulation.
    pub fn all() -> impl Iterator<Item = NodeId> {
        (0..with_context(|cx| cx.next_node_id.0)).map(NodeId)
    }
}
