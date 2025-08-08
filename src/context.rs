use crate::event::{Event, EventHandler};
use crate::simulator::Simulator;
use crate::time::TimeScheduler;
use async_task::{Runnable, Task};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;
use std::time::Duration;

thread_local! {
    static CONTEXT: Context2 = const {Context2{
        context:RefCell::new(None),
        ready_queue:RefCell::new(VecDeque::new()),
        rng:RefCell::new(None),
        time:Cell::new(None),
    }};
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    Context2::with(|c| f(&mut c.context.borrow_mut()))
}

pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}

pub struct Context {
    pub current_node: NodeId,
    next_node_id: NodeId,
    event_handler: Box<dyn EventHandler>,
    pub time_scheduler: TimeScheduler,
    pub simulators: HashMap<TypeId, Rc<RefCell<dyn Simulator>>>,
}

pub struct Context2 {
    context: RefCell<Option<Context>>,
    ready_queue: RefCell<VecDeque<Runnable>>,
    pub rng: RefCell<Option<ChaCha12Rng>>,
    pub time: Cell<Option<Duration>>,
}

impl Context2 {
    pub fn with<R>(f: impl FnOnce(&Context2) -> R) -> R {
        CONTEXT.with(f)
    }
}

#[derive(Eq, Debug, PartialEq, Clone, Copy)]
pub struct NodeId(usize);

impl NodeId {
    pub const INIT: Self = NodeId(0);
}

impl Context {
    pub fn run(
        event_handler: Box<dyn EventHandler>,
        seed: u64,
        start_time: Duration,
        init_fn: Box<dyn FnOnce() + '_>,
    ) -> Box<dyn EventHandler> {
        Context2::with(
            |Context2 {
                 context,
                 ready_queue,
                 time,
                 rng,
             }| {
                assert!(time.replace(Some(start_time)).is_none());
                assert!(
                    rng.replace(Some(ChaCha12Rng::seed_from_u64(seed)))
                        .is_none()
                );
                assert!(ready_queue.borrow_mut().is_empty());
                let new_context = Context {
                    current_node: NodeId::INIT,
                    next_node_id: NodeId(1),
                    event_handler,
                    time_scheduler: TimeScheduler::new(),
                    // random is already deterministic at this point.
                    simulators: HashMap::new(),
                };
                assert!(context.borrow_mut().replace(new_context).is_none());
            },
        );
        init_fn();
        loop {
            while let Some(runnable) = Context2::with(|cx| {
                cx.context
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .record_event(Event::FuturePolled);
                cx.ready_queue.borrow_mut().pop_front()
            }) {
                runnable.run();
            }
            if !Context2::with(|cx2| {
                let mut cx = cx2.context.borrow_mut();
                let cx = cx.as_mut().unwrap();
                cx.time_scheduler
                    .wait_until_next_future_ready(&cx2.time, &mut *cx.event_handler)
            }) {
                break;
            }
        }
        CONTEXT
            .with(
                |Context2 {
                     context,
                     ready_queue,
                     rng,
                     time,
                 }| {
                    debug_assert!(ready_queue.borrow_mut().is_empty());
                    time.take().unwrap();
                    rng.take().unwrap();
                    context.borrow_mut().take().unwrap()
                },
            )
            .event_handler
    }

    pub fn record_event(&mut self, event: Event) {
        self.event_handler.handle_event(event);
    }

    pub fn new_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.event_handler.handle_event(Event::NodeSpawned(id));
        self.next_node_id.0 += 1;
        id
    }
    pub fn spawn<F: Future + 'static>(&mut self, node: NodeId, future: F) -> Task<F::Output> {
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
        Context2::with(|cx| cx.ready_queue.borrow_mut().push_back(f));
    }
}

pin_project_lite::pin_project! {
    pub(crate) struct NodeFuture<F:Future> {
        id: NodeId ,
        #[pin]
        inner: F,
    }
}

impl<F: Future> Future for NodeFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        with_context(|cx| {
            assert!(cx.current_node == NodeId::INIT);
            cx.current_node = *this.id;
        });
        let result = this.inner.poll(cx);
        with_context(|cx| {
            debug_assert_eq!(cx.current_node, *this.id);
            cx.current_node = NodeId::INIT;
        });
        result
    }
}
