use std::future::Future;
use std::sync::{Arc, Mutex};
use reduced_rand::SeedableRng;
use reduced_rand_chacha::ChaCha12Rng;
use crate::event::{EventHandler, NoopEventHandler};
use crate::network::{DefaultNetwork, Network};
use crate::node::{NodeId, NodeIdSupplier, NODES};
use crate::task::{Executor, Task};

pub static CONTEXT: Mutex<Option<Context>> = Mutex::new(None);

pub struct Context {
    pub(crate) executor: Arc<Executor>,
    pub(crate) node_id_supplier: NodeIdSupplier,
    pub(crate) current_node: Option<NodeId>,
    pub(crate) event_handler: Box<dyn EventHandler + Send>,
    pub random_generator: ChaCha12Rng,
    pub network: Arc<dyn Network + Send + Sync>,
}

#[allow(dead_code)]
pub fn run(future: impl Future<Output = ()> + Send + Sync + 'static) {
    let executor = Arc::new(Executor::new(false));

    {
        let mut context_option = CONTEXT.lock().unwrap();
        match *context_option {
            Some(_) => panic!("Cannot create new context: Context already exists."),
            None => {
                executor.queue(Arc::new(Task::new(future)));
                *context_option = Some(Context {
                    executor: executor.clone(),
                    node_id_supplier: NodeIdSupplier::new(),
                    current_node: None,
                    network: Arc::new(DefaultNetwork{}),
                    event_handler: Box::new(NoopEventHandler),
                    random_generator: ChaCha12Rng::seed_from_u64(0u64),
                });
            }
        }
    }

    executor.run();
    *CONTEXT.lock().unwrap() = None;
    *NODES.lock().unwrap() = Vec::new();
}

#[allow(dead_code)]
pub fn simulate(future: impl Future<Output = ()> + Send + Sync + 'static) {
    let executor = Arc::new(Executor::new(true));

    {
        let mut context_option = CONTEXT.lock().unwrap();
        match *context_option {
            Some(_) => panic!("Cannot create new context: Context already exists."),
            None => {
                executor.queue(Arc::new(Task::new(future)));
                *context_option = Some(Context {
                    executor: executor.clone(),
                    node_id_supplier: NodeIdSupplier::new(),
                    current_node: None,
                    network: Arc::new(DefaultNetwork{}),
                    event_handler: Box::new(NoopEventHandler),
                    random_generator: ChaCha12Rng::seed_from_u64(0u64),
                });
            }
        }
    }

    executor.run();
    *CONTEXT.lock().unwrap() = None;
    *NODES.lock().unwrap() = Vec::new();
}
