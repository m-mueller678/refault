use crate::check_send::{CheckSend, Constraint, SimBound};
use crate::event::Event;
use crate::simulator::for_all_simulators;
use crate::{
    context::time::TimeScheduler,
    context::{Context2, with_context},
};
use cooked_waker::{IntoWaker, WakeRef};
use futures_channel::oneshot;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::io;
use std::sync::atomic::Ordering::Relaxed;
use std::task::Poll;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, atomic::AtomicUsize},
    task::Context,
};

use super::id::Id;

pub struct ExecutorQueue {
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

    pub fn executor(&self) -> Executor {
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

pub struct Executor {
    // each entry corresponds to one TaskShared
    // entries are None while the task is executing, cancelled, or completed
    #[allow(clippy::type_complexity)]
    tasks: HashMap<Id, TaskEntry>,
    final_stopped: bool,
    nodes: Vec<NodeData>,
    pub time_scheduler: TimeScheduler,
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
    /// Awaiting it will return the value returned by the spawned future or `None` if the task was aborted.
    ///
    /// A task handle must either be polled to completion or destroyd via [detach](Self::detach) or [abort](Self::abort).
    /// Dropping the handle without doing any of those will panic.
    #[must_use]
    pub struct TaskHandle<T> {
        #[pin]
        result: oneshot::Receiver<T>,
        // this is always some until the handle is consumed via detach or abort.
        abort: Option<AbortHandle>,
    }

    impl<T> PinnedDrop for TaskHandle<T> {
        fn drop(this: Pin<&mut Self>) {
            if let Some(abort)=&this.abort{
                abort.abort_inner();
            }
        }
    }
}

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
    pub fn abort(mut self) {
        self.abort.take().unwrap().abort();
    }

    /// Detach this handle from the task, allowing the task to keep running.
    pub fn detach(mut self) {
        self.abort.take().unwrap();
    }

    /// Obtain a handle that can be used to abort the associated task.
    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.as_ref().unwrap().clone()
    }
}

/// A handle that can be used to abort a task.
///
/// Dropping this will not abort the task.
#[derive(Clone)]
pub struct AbortHandle(CheckSend<Arc<TaskShared>, SimBound>);

const TASK_CANCELLED: usize = 0;
const TASK_READY: usize = 1;
const TASK_WAITING: usize = 2;
const TASK_COMPLETE: usize = 3;
const TASK_END: usize = 4;

struct TaskShared {
    state: AtomicUsize,
    id: Id,
    node: NodeId,
}

impl WakeRef for TaskShared {
    fn wake_by_ref(&self) {
        Context2::with(|cx| {
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
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_final_sopping(&self) -> bool {
        self.final_stopped
    }

    pub fn final_stop() {
        with_context(|cx| cx.executor.final_stopped = true);
        for n in NodeId::all() {
            n.stop_inner(true);
        }
    }

    fn spawn<F: Future + 'static>(&mut self, node: NodeId, future: F) -> TaskHandle<F::Output> {
        match self.nodes[node.0].run_level {
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
            abort: Some(AbortHandle(SimBound::wrap(task_entry.shared.clone()))),
        };
        self.tasks.insert(task_id, task_entry);
        self.nodes[node.0].tasks.insert(task_id);
        <TaskShared as WakeRef>::wake_by_ref(&task_handle.abort.as_ref().unwrap().0);
        task_handle
    }

    fn remove_task_entry(&mut self, task_id: Id) {
        let removed = self.tasks.remove(&task_id);
        let task_entry = removed.unwrap();
        debug_assert_eq!(task_entry.shared.id, task_id);
        assert!(task_entry.task.into_inner().is_none());
        let node = &mut self.nodes[task_entry.shared.node.0];
        let removed = node.tasks.remove(&task_id);
        debug_assert!(removed);
    }

    pub fn run_current_context(cx2: &Context2) {
        loop {
            let Some(mut task) = cx2.with_cx(|cx| {
                loop {
                    let task_id = cx2
                        .queue
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .ready_queue
                        .pop_front()?;
                    let task_entry = cx.executor.tasks.get_mut(&task_id).unwrap();
                    match task_entry.shared.state.load(Relaxed) {
                        TASK_READY => {
                            let task = task_entry.task.take().unwrap();
                            cx.event_handler
                                .handle_event(Event::TaskRun(task_entry.shared.id));
                            task_entry.shared.state.store(TASK_WAITING, Relaxed);
                            return Some(task);
                        }
                        TASK_CANCELLED => {
                            let id = task_entry.shared.id;
                            cx.executor.remove_task_entry(id);
                            continue;
                        }
                        TASK_COMPLETE | TASK_WAITING | TASK_END.. => unreachable!(),
                    }
                }
            }) else {
                if cx2.with_cx(|cx| {
                    cx.executor
                        .time_scheduler
                        .wait_until_next_future_ready(&cx2.time, &mut *cx.event_handler)
                }) {
                    continue;
                } else {
                    break;
                }
            };
            let &TaskShared { node, id, .. } = &**task.as_base();
            cx2.node_scope(node, move || {
                let keep = task.as_mut().run();
                cx2.with_cx(|cx| {
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

    fn abort_inner(&self) {
        let this = &*self.0;
        (Context2::with_in_node(this.node, |cx| {
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
    NodeId::current().spawn(future)
}

/// A unique identifier for a node within a simulation.
#[derive(Eq, Hash, Debug, PartialEq, Clone, Copy)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    pub(crate) const INIT: Self = NodeId(0);

    /// Create a new node that tasks can be run on.
    ///
    /// Can only be called from within a simulation.
    pub fn create_node() -> NodeId {
        Context2::with(|cx2| {
            let id = cx2.with_cx(|cx| {
                let id = NodeId(cx.executor.nodes.len());
                assert!(
                    !cx.executor.final_stopped,
                    "node spawn during simulation shutdown"
                );
                cx.executor.nodes.push(NodeData {
                    run_level: NodeRunLevel::Running,
                    tasks: HashSet::new(),
                });
                cx.event_handler.handle_event(Event::NodeSpawned(id));
                id
            });
            cx2.node_scope(id, || {
                for_all_simulators(cx2, true, |s| {
                    s.create_node();
                });
            });
            id
        })
    }

    /// Spawn a task on this node.
    ///
    /// Can only be called from within a simulation.
    pub fn spawn<F: Future + 'static>(self, future: F) -> TaskHandle<F::Output> {
        with_context(|cx| {
            cx.event_handler.handle_event(Event::TaskSpawned);
            cx.executor.spawn(self, future)
        })
    }

    /// Returns the id of the node this task is running on.
    ///
    /// Can only be called from within a simulation.
    pub fn current() -> Self {
        Self::try_current().expect("not inside a simulation")
    }

    /// Returns the id of the current node if within the simulation.
    /// Returns `None` otherwise.
    pub fn try_current() -> Option<Self> {
        Context2::with(|cx2| {
            if cx2.time.get().is_some() {
                Some(cx2.current_node())
            } else {
                None
            }
        })
    }

    /// Invoke Simulator::stop on all simulators and stop all tasks on this node.
    ///
    /// Attempting to spawn tasks on a stopped node will panic.
    /// Attempting to stop a node that is already stopped does nothing.
    pub fn stop(self) {
        self.stop_inner(false);
    }

    fn stop_inner(self, is_final: bool) {
        Context2::with_in_node(self, |cx2| {
            let was_running = cx2.with_cx(|cx| {
                let node = &mut cx.executor.nodes[self.0];
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
                cx2.with_cx(|context| {
                    let node = &mut context.executor.nodes[self.0];
                    node.tasks.iter().copied().collect::<Vec<Id>>()
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
            for_all_simulators(cx2, false, |x| x.stop_node());
        });
    }

    /// Iterate over all nodes in the current simulation.
    pub fn all() -> impl Iterator<Item = NodeId> {
        (0..with_context(|cx| cx.executor.nodes.len())).map(NodeId)
    }
}
