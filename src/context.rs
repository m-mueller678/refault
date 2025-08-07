use crate::event::{Event, EventHandler, record_event};
use crate::time::TimeScheduler;
use async_task::{Runnable, Task};
use rand_chacha::ChaCha12Rng;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
thread_local! {

static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    CONTEXT.with(|c| f(&mut c.borrow_mut()))
}
pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}
pub struct Context {
    pub current_node: Option<NodeId>,
    pub ready_queue: VecDeque<Runnable>,
    pub event_handler: Box<dyn EventHandler>,
    pub random_generator: ChaCha12Rng,
    pub simulation_start_time: u64,
    pub nodes: Vec<Node>,
    pub time_scheduler: TimeScheduler,
}
#[derive(Eq, Debug, PartialEq, Clone, Copy)]
pub struct NodeId(usize);
pub struct Node {}

impl Context {
    pub fn run(self) -> Self {
        with_context_option(|c| assert!(c.replace(self).is_none()));
        loop {
            while let Some(runnable) = with_context(|cx| cx.ready_queue.pop_front()) {
                record_event(Event::FuturePolledEvent);
                runnable.run();
            }
            if !with_context(|cx| cx.time_scheduler.wait_until_next_future_ready()) {
                break;
            }
        }
        let context = with_context_option(|c| c.take().unwrap());
        context
    }
    pub fn new_node(&mut self) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.event_handler.handle_event(Event::NodeSpawnedEvent(id));
        self.nodes.push(Node {});
        id
    }
    pub fn spawn<F: Future + 'static>(
        &mut self,
        node: Option<NodeId>,
        future: F,
    ) -> Task<F::Output> {
        self.event_handler.handle_event(Event::TaskSpawnedEvent);
        let fut = NodeFuture {
            id: node,
            inner: future,
        };
        let (runnable, task) = async_task::spawn_local(fut, Self::schedule);
        self.ready_queue.push_back(runnable);
        task
    }
    fn schedule(f: Runnable) {
        with_context(|cx| {
            cx.ready_queue.push_back(f);
        })
    }
}

pin_project_lite::pin_project! {
    pub(crate) struct NodeFuture<F:Future> {
        id: Option<NodeId>,
        #[pin]
        inner: F,
    }
}

impl<F: Future> Future for NodeFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        with_context(|cx| {
            assert!(cx.current_node.is_none());
            cx.current_node = *this.id;
        });
        let result = this.inner.poll(cx);
        with_context(|cx| {
            debug_assert_eq!(cx.current_node, *this.id);
            cx.current_node = None;
        });
        result
    }
}
