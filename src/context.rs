use crate::event::EventHandler;
use crate::executor::Executor;
use crate::network::Network;
use crate::node::{Node, NodeId, NodeIdSupplier};
use rand_chacha::ChaCha12Rng;
use std::cell::RefCell;
use std::sync::Arc;
thread_local! {

static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    CONTEXT.with(|c| f(&mut c.borrow_mut()))
}
pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}
pub fn run_with_context<R>(context: Context, f: impl FnOnce() -> R) -> (R, Context) {
    with_context_option(|c| assert!(c.replace(context).is_none()));
    let r = f();
    let context = with_context_option(|c| c.take().unwrap());
    (r, context)
}
pub struct Context {
    pub executor: Arc<Executor>,
    pub node_id_supplier: NodeIdSupplier,
    pub current_node: Option<NodeId>,
    pub event_handler: Box<dyn EventHandler>,
    pub random_generator: ChaCha12Rng,
    pub simulation_start_time: u64,
    pub network: Arc<dyn Network + Send + Sync>,
    pub nodes: Vec<Node>,
}
