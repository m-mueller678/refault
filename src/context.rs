use crate::event::EventHandler;
use crate::executor::Executor;
use crate::network::Network;
use crate::node::{NodeId, NodeIdSupplier};
use rand_chacha::ChaCha12Rng;
use std::sync::{Arc, Mutex};

static CONTEXT: Mutex<Option<Context>> = Mutex::new(None);

pub(crate) fn with_context<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    let mut context = CONTEXT.lock().unwrap();
    f(&mut *context)
}
pub struct Context {
    pub(crate) executor: Arc<Executor>,
    pub(crate) node_id_supplier: NodeIdSupplier,
    pub(crate) current_node: Option<NodeId>,
    pub(crate) event_handler: Box<dyn EventHandler + Send>,
    pub random_generator: ChaCha12Rng,
    pub simulation_start_time: u64,
    pub network: Arc<dyn Network + Send + Sync>,
}
