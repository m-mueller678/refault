pub mod executor;
pub mod id;
pub mod time;

use crate::event::EventHandler;
use crate::simulator::Simulator;
use executor::{Executor, ExecutorQueue, NodeId};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use scopeguard::guard;
use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::Duration;

thread_local! {
    static CONTEXT: Context2 = const {Context2{
        context:RefCell::new(None),
        rng:RefCell::new(None),
        time:Cell::new(None),
        queue:RefCell::new(None),
        pre_next_global_id:Cell::new(0),
        current_node:Cell::new(NodeId::INIT),
    }};
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    Context2::with(|c| f(&mut c.context.borrow_mut()))
}

pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}

pub type SimulatorRc = Option<Box<dyn Simulator>>;

pub struct Context {
    pub executor: Executor,
    pub event_handler: Box<dyn EventHandler>,
    pub simulators_by_type: HashMap<TypeId, usize>,
    pub simulators: Vec<SimulatorRc>,
}

pub struct Context2 {
    //TODO Context anchor object to ensure objects are not moved between contexts
    pub context: RefCell<Option<Context>>,
    pub rng: RefCell<Option<ChaCha12Rng>>,
    pub time: Cell<Option<Duration>>,
    pub queue: RefCell<Option<ExecutorQueue>>,
    current_node: Cell<NodeId>,
    pre_next_global_id: Cell<u64>,
}

impl Context2 {
    pub fn with<R>(f: impl FnOnce(&Context2) -> R) -> R {
        CONTEXT.with(f)
    }

    pub fn node_scope<R>(&self, node: NodeId, f: impl FnOnce() -> R) -> R {
        let calling_node = self.current_node.get();
        self.current_node.set(node);
        let _guard = guard((), |()| self.current_node.set(calling_node));
        f()
    }

    pub fn with_in_node<R>(node: NodeId, f: impl FnOnce(&Context2) -> R) -> R {
        Self::with(|cx| cx.node_scope(node, || f(cx)))
    }

    pub fn current_node(&self) -> NodeId {
        self.current_node.get()
    }

    pub fn with_cx<R>(&self, f: impl FnOnce(&mut Context) -> R) -> R {
        f(self.context.borrow_mut().as_mut().unwrap())
    }
}

pub struct ContextInstallGuard(NotSendSync);

impl ContextInstallGuard {
    pub fn new(event_handler: Box<dyn EventHandler>, seed: u64, start_time: Duration) -> Self {
        Context2::with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            assert!(cx2.time.replace(Some(start_time)).is_none());
            assert!(
                cx2.rng
                    .replace(Some(ChaCha12Rng::seed_from_u64(seed)))
                    .is_none()
            );
            assert!(
                cx2.queue
                    .borrow_mut()
                    .replace(ExecutorQueue::new())
                    .is_none()
            );
            let new_context = Context {
                executor: cx2.queue.borrow_mut().as_mut().unwrap().executor(),
                event_handler,
                // random is already deterministic at this point.
                simulators: Vec::new(),
                simulators_by_type: HashMap::new(),
            };
            assert!(cx2.context.borrow_mut().replace(new_context).is_none());
            assert!(cx2.pre_next_global_id.get() == 0);
        });
        ContextInstallGuard(NotSendSync::default())
    }

    pub fn destroy(&mut self) -> Option<Box<dyn EventHandler>> {
        CONTEXT.with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            cx2.time.get()?;
            assert!(cx2.queue.borrow().as_ref().unwrap().none_ready() || std::thread::panicking());
            Executor::final_stop();
            while let Some(x) = cx2.with_cx(|cx| cx.simulators.pop()) {
                drop(x.unwrap())
            }
            let Context { event_handler, .. } = cx2.context.take().unwrap();
            cx2.time.take().unwrap();
            cx2.rng.take().unwrap();
            Some(event_handler)
        })
    }
}

impl Drop for ContextInstallGuard {
    fn drop(&mut self) {
        self.destroy();
    }
}

pub type NotSendSync = PhantomData<*const ()>;
