use crate::event::{Event, EventHandler};
use crate::time::TimeScheduler;
use async_task::{Runnable, Task};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

thread_local! {
    static CONTEXT: Context2 = const {Context2{
        context:RefCell::new(None),
        ready_queue:RefCell::new(VecDeque::new()),
    }};
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    CONTEXT.with(|c| f(&mut c.context.borrow_mut()))
}
pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}
pub struct Context {
    pub current_node: Option<NodeId>,
    event_handler: Box<dyn EventHandler>,
    pub random_generator: ChaCha12Rng,
    nodes: Vec<Node>,
    pub time_scheduler: Option<TimeScheduler>,
    time: Duration,
}
struct Context2 {
    context: RefCell<Option<Context>>,
    ready_queue: RefCell<VecDeque<Runnable>>,
}
#[derive(Eq, Debug, PartialEq, Clone, Copy)]
pub struct NodeId(usize);
pub struct Node {}

impl Context {
    pub fn time(&self) -> Duration {
        self.time
    }

    pub fn run(
        event_handler: Box<dyn EventHandler>,
        seed: u64,
        start_time: Duration,
        init_fn: Box<dyn FnOnce() + '_>,
    ) -> Box<dyn EventHandler> {
        let context = Context {
            current_node: None,
            event_handler,
            random_generator: ChaCha12Rng::seed_from_u64(seed),
            time: start_time,
            nodes: Vec::new(),
            time_scheduler: None,
        };
        CONTEXT.with(|cx| {
            assert!(cx.context.borrow_mut().replace(context).is_none());
            assert!(cx.ready_queue.borrow_mut().is_empty());
        });
        let time_scheduler = TimeScheduler::new();
        with_context(|cx| cx.time_scheduler = Some(time_scheduler));
        init_fn();
        loop {
            while let Some(runnable) = CONTEXT.with(|cx| {
                cx.context
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .record_event(Event::FuturePolled);
                cx.ready_queue.borrow_mut().pop_front()
            }) {
                runnable.run();
            }
            if !with_context(|cx| {
                cx.time_scheduler
                    .as_mut()
                    .unwrap()
                    .wait_until_next_future_ready(&mut cx.time, &mut *cx.event_handler)
            }) {
                break;
            }
        }
        CONTEXT
            .with(|cx| {
                debug_assert!(cx.ready_queue.borrow_mut().is_empty());
                cx.context.borrow_mut().take().unwrap()
            })
            .event_handler
    }

    pub fn record_event(&mut self, event: Event) {
        self.event_handler.handle_event(event);
    }

    pub fn new_node(&mut self) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.event_handler.handle_event(Event::NodeSpawned(id));
        self.nodes.push(Node {});
        id
    }
    pub fn spawn<F: Future + 'static>(
        &mut self,
        node: Option<NodeId>,
        future: F,
    ) -> Task<F::Output> {
        self.event_handler.handle_event(Event::TaskSpawned);
        let fut = NodeFuture {
            id: node,
            inner: future,
        };
        let (runnable, task) = async_task::spawn_local(fut, Self::schedule);
        Self::schedule(runnable);
        task
    }
    fn schedule(f: Runnable) {
        CONTEXT.with(|cx| cx.ready_queue.borrow_mut().push_back(f));
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
