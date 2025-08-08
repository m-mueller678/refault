use crate::event::{Event, EventHandler};
use crate::executor::{Executor, ExecutorQueue};
use crate::simulator::Simulator;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    static CONTEXT: Context2 = const {Context2{
        context:RefCell::new(None),
        rng:RefCell::new(None),
        time:Cell::new(None),
        queue:RefCell::new(None),
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
    pub executor: Executor,
    pub event_handler: Box<dyn EventHandler>,
    pub simulators: HashMap<TypeId, Rc<RefCell<dyn Simulator>>>,
}

pub struct Context2 {
    pub context: RefCell<Option<Context>>,
    pub rng: RefCell<Option<ChaCha12Rng>>,
    pub time: Cell<Option<Duration>>,
    pub queue: RefCell<Option<ExecutorQueue>>,
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
                 time,
                 rng,
                 queue,
             }| {
                assert!(time.replace(Some(start_time)).is_none());
                assert!(
                    rng.replace(Some(ChaCha12Rng::seed_from_u64(seed)))
                        .is_none()
                );
                assert!(queue.borrow_mut().replace(ExecutorQueue::new()).is_none());
                let new_context = Context {
                    current_node: NodeId::INIT,
                    executor: queue.borrow_mut().as_mut().unwrap().executor(),
                    next_node_id: NodeId(1),
                    event_handler,
                    // random is already deterministic at this point.
                    simulators: HashMap::new(),
                };
                assert!(context.borrow_mut().replace(new_context).is_none());
            },
        );
        init_fn();
        Executor::run_current_context();
        let context = CONTEXT.with(
            |Context2 {
                 context,
                 queue,
                 rng,
                 time,
             }| {
                time.take().unwrap();
                rng.take().unwrap();
                assert!(queue.take().unwrap().is_empty());
                context.borrow_mut().take().unwrap()
            },
        );
        context.event_handler
    }

    pub fn new_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.event_handler.handle_event(Event::NodeSpawned(id));
        self.next_node_id.0 += 1;
        id
    }
}
