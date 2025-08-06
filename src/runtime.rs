use crate::context::{Context, run_with_context};
use crate::event::Event;
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
use crate::executor::{Executor, Task};
use crate::network::{DefaultNetwork, Network};
use crate::node::NodeIdSupplier;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::sync::Arc;
use std::thread;

pub struct Runtime {
    seed: u64,
    simulation_start_time: u64,
    network: Arc<dyn Network + Send + Sync + 'static>,
    fast_forward_time: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: 1750363615882u64,
            network: Arc::new(DefaultNetwork {}),
            fast_forward_time: true,
        }
    }
}

impl Runtime {
    pub fn with_network(mut self, net: Arc<dyn Network + Send + 'static + Sync>) -> Self {
        self.network = net;
        self
    }
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
    pub fn with_simulation_start_time(mut self, simulation_start_time: u64) -> Self {
        self.simulation_start_time = simulation_start_time;
        self
    }
    pub fn with_fast_forward_time(mut self, fast_forward_time: bool) -> Self {
        self.fast_forward_time = fast_forward_time;
        self
    }
    fn validate_events(
        &self,
        future: impl Future<Output = ()> + Send + Sync + 'static,
        events: Arc<Vec<Event>>,
    ) {
        let events_size = events.len();
        let event_handler = ValidatingEventHandler::new(events);
        let next_event_index = event_handler.next_event_index;
        self.run_simulation(future, Box::new(event_handler));
    }

    pub fn run(&self, f: impl 'static + Send + Sync + Future<Output = ()>) {
        self.run_simulation(f, Box::new(NoopEventHandler));
    }
    pub fn check_determinism<T: Future<Output = ()> + Send + Sync + 'static>(
        &self,
        mut future_producer: impl FnMut() -> T,
        iterations: usize,
    ) {
        assert!(iterations > 1);
        let event_handler = RecordingEventHandler::new();
        let events = Arc::new(self.run_simulation(future_producer(), Box::new(event_handler)));
        for _ in 1..iterations {
            self.validate_events(future_producer(), events.clone());
        }
    }

    fn run_simulation(
        &self,
        future: impl Future<Output = ()> + Send + Sync + 'static,
        event_handler: Box<dyn EventHandler>,
    ) -> Vec<Event> {
        // Run the simulation on a new thread to avoid thread local state on this thread interfering
        // with random number generation
        thread::scope(|scope| {
            scope
                .spawn(move || {
                    let executor = Arc::new(Executor::new(self.fast_forward_time));
                    executor.queue(Arc::new(Task::new(future)));
                    let ((), context) = run_with_context(
                        self.make_context(executor.clone(), event_handler),
                        || {
                            executor.run();
                        },
                    );
                    context.event_handler.finalize()
                })
                .join()
                .unwrap()
        })
    }

    fn make_context(
        &self,
        executor: Arc<Executor>,
        event_handler: Box<dyn EventHandler>,
    ) -> Context {
        Context {
            executor: executor.clone(),
            node_id_supplier: NodeIdSupplier::new(),
            current_node: None,
            network: self.network.clone(),
            event_handler,
            random_generator: ChaCha12Rng::seed_from_u64(self.seed),
            simulation_start_time: self.simulation_start_time,
            nodes: Vec::new(),
        }
    }
}

struct ExecutionOptions {
    fast_forward_time: bool,
    event_handler: Box<dyn EventHandler + Send>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            fast_forward_time: true,
            event_handler: Box::new(NoopEventHandler),
        }
    }
}
