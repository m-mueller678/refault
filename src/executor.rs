//! Functions for handling tasks.
use crate::SimCxl;
use crate::event::Event;
use crate::id::Id;
use crate::node_id::NodeId;
use crate::simulator::for_all_simulators;
use crate::{SimCx, time::TimeScheduler};
use cooked_waker::{IntoWaker, WakeRef};
use futures::channel::oneshot;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::io;
use std::marker::PhantomData;
use std::sync::atomic::Ordering::Relaxed;
use std::task::Poll;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, atomic::AtomicUsize},
    task::Context,
};

pub(crate) struct ExecutorQueue {
    ready_queue: VecDeque<Id>,
}

impl ExecutorQueue {
    pub fn none_ready(&self) -> bool {
        self.ready_queue.is_empty()
    }

    pub fn new() -> Self {
        ExecutorQueue {
            ready_queue: VecDeque::new(),
        }
    }

    pub(crate) fn executor(&self) -> Executor {
        Executor {
            final_stopped: false,
            tasks: HashMap::new(),
            nodes: vec![NodeData {
                run_level: NodeRunLevel::Running,
                tasks: HashSet::new(),
            }],
            time_scheduler: TimeScheduler::new(),
        }
    }
}

pub(crate) struct Executor {
    // each entry corresponds to one TaskShared
    // entries are None while the task is executing, cancelled, or completed
    #[allow(clippy::type_complexity)]
    tasks: HashMap<Id, TaskEntry>,
    final_stopped: bool,
    nodes: Vec<NodeData>,
    pub(crate) time_scheduler: TimeScheduler,
}

enum NodeRunLevel {
    Running,
    Stopped,
    FinalStopped,
}

struct NodeData {
    run_level: NodeRunLevel,
    tasks: HashSet<Id>,
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

// TODO this could probably be just a dyn FUture with shared state kepy outside.
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
    /// Awaiting it will return the value returned by the spawned future or a [TaskAborted] error if the task was aborted.
    ///
    /// A task handle should either be polled to completion or destroyd via [detach](Self::detach) or [abort](Self::abort).
    /// Dropping the handle will implicitly detach it.
    /// Explicitly detaching is preferrable to communicate intent.
    #[must_use]
    pub struct TaskHandle<T> {
        #[pin]
        result: oneshot::Receiver<T>,
        // this is always some until the handle is consumed via detach or abort.
        abort: AbortHandle,
    }
}

/// The error returned from a [TaskHandle] if the associated task was aborted.
#[derive(Debug)]
pub struct TaskAborted(());

impl From<TaskAborted> for io::Error {
    fn from(_value: TaskAborted) -> Self {
        io::Error::other("aborted")
    }
}

impl Display for TaskAborted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self, f)
    }
}

impl Error for TaskAborted {}

impl<T> TaskHandle<T> {
    /// Abort the associated task.
    pub fn abort(self) {
        self.abort.abort_inner();
    }

    /// Detach this handle from the task, allowing the task to keep running.
    pub fn detach(self) {}

    /// Obtain a handle that can be used to abort the associated task.
    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }
}

/// A handle that can be used to abort a task.
///
/// Dropping this will not abort the task.
#[derive(Clone)]
pub struct AbortHandle {
    shared: Arc<TaskShared>,
    _unsend: PhantomData<*const u8>,
}

/// A handle that aborts the associated task when dropped.
#[derive(Clone)]
pub struct AbortGuard(AbortHandle);

impl Drop for AbortGuard {
    fn drop(&mut self) {
        self.0.abort_inner();
    }
}

const TASK_CANCELLED: usize = 0;
const TASK_READY: usize = 1;
const TASK_WAITING: usize = 2;
const TASK_COMPLETE: usize = 3;
const TASK_END: usize = 4;

struct TaskShared {
    state: AtomicUsize,
    id: Id,
    node: crate::node_id::NodeId,
}

impl WakeRef for TaskShared {
    fn wake_by_ref(&self) {
        SimCx::with(|cx| {
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

/// Spawn a task on the given node and return a handle.
///
/// This is a low level interface, that can be misused to unintentionally send data between nodes.
/// Prefer using [NodeId::spawn] or [spawn] instead.
pub fn spawn_task_on_node<F: Future + 'static>(
    node: crate::node_id::NodeId,
    future: F,
) -> TaskHandle<F::Output> {
    SimCxl::with(|cx| {
        cx.event_handler.handle_event(Event::TaskSpawned);
        cx.executor.spawn(node, future)
    })
}

impl Executor {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_final_sopping(&self) -> bool {
        self.final_stopped
    }

    pub fn final_stop() {
        SimCxl::with(|cx| cx.executor.final_stopped = true);
        for node in NodeId::all() {
            stop_node(node, true);
        }
    }

    pub fn push_new_node(&mut self) -> crate::node_id::NodeId {
        let id = NodeId::from_index(self.node_count());
        assert!(!self.final_stopped, "node spawn during simulation shutdown");
        self.nodes.push(NodeData {
            run_level: NodeRunLevel::Running,
            tasks: HashSet::new(),
        });
        id
    }

    fn spawn<F: Future + 'static>(
        &mut self,
        node: crate::node_id::NodeId,
        future: F,
    ) -> TaskHandle<F::Output> {
        match self.nodes[node.0.get() - 1].run_level {
            NodeRunLevel::Running => (),
            NodeRunLevel::Stopped | NodeRunLevel::FinalStopped => {
                panic!("node {node:?} is stopped")
            }
        }
        let (snd, rcv) = oneshot::channel();
        let task_id = Id::new();
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
            abort: AbortHandle {
                shared: task_entry.shared.clone(),
                _unsend: PhantomData,
            },
        };
        self.tasks.insert(task_id, task_entry);
        self.nodes[node.0.get() - 1].tasks.insert(task_id);
        <TaskShared as WakeRef>::wake_by_ref(&task_handle.abort.shared);
        task_handle
    }

    fn remove_task_entry(&mut self, task_id: Id) {
        let removed = self.tasks.remove(&task_id);
        let task_entry = removed.unwrap();
        debug_assert_eq!(task_entry.shared.id, task_id);
        assert!(task_entry.task.into_inner().is_none());
        let node = &mut self.nodes[task_entry.shared.node.0.get() - 1];
        let removed = node.tasks.remove(&task_id);
        debug_assert!(removed);
    }

    pub(crate) fn run_current_context(cx: &SimCx) {
        loop {
            let Some(mut task) = cx.with_cx(|cxl| {
                loop {
                    let task_id = cx
                        .queue
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .ready_queue
                        .pop_front()?;
                    let task_entry = cxl.executor.tasks.get_mut(&task_id).unwrap();
                    match task_entry.shared.state.load(Relaxed) {
                        TASK_READY => {
                            let task = task_entry.task.take().unwrap();
                            cxl.event_handler
                                .handle_event(Event::TaskRun(task_entry.shared.id));
                            task_entry.shared.state.store(TASK_WAITING, Relaxed);
                            return Some(task);
                        }
                        TASK_CANCELLED => {
                            let id = task_entry.shared.id;
                            cxl.executor.remove_task_entry(id);
                            continue;
                        }
                        TASK_COMPLETE | TASK_WAITING | TASK_END.. => unreachable!(),
                    }
                }
            }) else {
                if cx.with_cx(|cxl| {
                    cxl.executor
                        .time_scheduler
                        .wait_until_next_future_ready(&cx.cxu().time, &mut *cxl.event_handler)
                }) {
                    continue;
                } else {
                    break;
                }
            };
            let &TaskShared { node, id, .. } = &**task.as_base();
            cx.node_scope(node, move || {
                let keep = task.as_mut().run();
                cx.with_cx(|cx| {
                    if keep {
                        match task.as_base().state.load(Relaxed) {
                            TASK_COMPLETE | TASK_END.. => unreachable!(),
                            TASK_CANCELLED => {
                                // task was put into queue by abort, so we should keep the entry in the map
                                drop(task);
                            }
                            TASK_WAITING | TASK_READY => {
                                let task_entry = cx.executor.tasks.get_mut(&id).unwrap();
                                assert!(task_entry.task.replace(Some(task)).is_none());
                            }
                        }
                    } else {
                        drop(task);
                        cx.executor.remove_task_entry(id);
                    }
                })
            });
        }
    }
}

fn abort_local(
    task_id: Id,
    queue: &mut ExecutorQueue,
    tasks: &mut HashMap<Id, TaskEntry>,
) -> Option<Pin<Box<dyn TaskDyn>>> {
    let task_entry = tasks.get_mut(&task_id)?;
    let state = task_entry.shared.state.load(Relaxed);
    match state {
        TASK_CANCELLED | TASK_COMPLETE => None,
        TASK_READY => {
            task_entry.shared.state.store(TASK_CANCELLED, Relaxed);
            task_entry.task.take()
        }
        TASK_WAITING => {
            task_entry.shared.state.store(TASK_CANCELLED, Relaxed);
            queue.ready_queue.push_back(task_id);
            task_entry.task.take()
        }
        TASK_END.. => unreachable!(),
    }
}

impl AbortHandle {
    /// Abort the associated task.
    pub fn abort(self) {
        self.abort_inner();
    }

    pub fn abort_on_drop(self) -> AbortGuard {
        AbortGuard(self)
    }

    fn abort_inner(&self) {
        let this = &*self.shared;
        (SimCx::with_in_node(this.node, |cx| {
            let task = abort_local(
                this.id,
                cx.queue.borrow_mut().as_mut().unwrap(),
                &mut cx.context.borrow_mut().as_mut().unwrap().executor.tasks,
            );
            // drop task after releasing locks in previus statement
            drop(task);
        }));
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Result<T, TaskAborted>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.result.poll(cx) {
            Poll::Ready(x) => Poll::Ready(x.map_err(|_| TaskAborted(()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Spawn a task on the current node.
pub fn spawn<F: Future + 'static>(future: F) -> TaskHandle<F::Output> {
    spawn_task_on_node(NodeId::current(), future)
}

pub(crate) fn stop_node(node: NodeId, is_final: bool) {
    SimCx::with_in_node(node, |cx| {
        let was_running = cx.with_cx(|cxl| {
            let node = &mut cxl.executor.nodes[node.to_index()];
            match node.run_level {
                NodeRunLevel::Running => {
                    node.run_level = if is_final {
                        NodeRunLevel::FinalStopped
                    } else {
                        NodeRunLevel::Stopped
                    };
                    true
                }
                NodeRunLevel::FinalStopped | NodeRunLevel::Stopped => {
                    if is_final {
                        node.run_level = NodeRunLevel::FinalStopped;
                    }
                    false
                }
            }
        });
        if !was_running {
            return;
        }
        let task_ids = {
            cx.with_cx(|context| {
                let node = &mut context.executor.nodes[node.to_index()];
                node.tasks.iter().copied().collect::<Vec<Id>>()
            })
        };
        for &task in &task_ids {
            drop(cx.with_cx(|cxl| {
                abort_local(
                    task,
                    cx.queue.borrow_mut().as_mut().unwrap(),
                    &mut cxl.executor.tasks,
                )
            }))
        }
        for_all_simulators(cx, false, |x| x.stop_node());
    });
}
