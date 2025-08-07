use crate::context::{Context, NodeId, with_context, with_context_option};
use crate::event::Event;
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
use crate::time::TimeScheduler;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::collections::VecDeque;
use std::sync::Arc;
use std::thread;

pub struct Runtime {
    seed: u64,
    simulation_start_time: u64,
    fast_forward_time: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: 1750363615882u64,
            fast_forward_time: true,
        }
    }
}

impl Runtime {
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
            self.run_simulation(
                future_producer(),
                Box::new(ValidatingEventHandler::new(events.clone())),
            );
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
                    let mut context = self.make_context(event_handler);
                    let task = context.spawn(None, future);
                    let context = context.run();
                    assert!(task.is_finished());
                    drop(task);
                    context.event_handler.finalize()
                })
                .join()
                .unwrap()
        })
    }

    fn make_context(&self, event_handler: Box<dyn EventHandler>) -> Context {
        Context {
            current_node: None,
            event_handler,
            random_generator: ChaCha12Rng::seed_from_u64(self.seed),
            simulation_start_time: self.simulation_start_time,
            nodes: Vec::new(),
            ready_queue: VecDeque::new(),
            time_scheduler: TimeScheduler::new(self.fast_forward_time),
        }
    }
}

pub fn current_node() -> Option<NodeId> {
    with_context(|cx| cx.current_node())
}

pub fn create_node() -> NodeId {
    with_context(|cx| cx.new_node())
}

pub fn is_running() -> bool {
    with_context_option(|cx| cx.is_some())
}
