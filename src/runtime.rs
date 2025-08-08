use crate::context::{Context, NodeId, with_context, with_context_option};
use crate::event::Event;
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
pub use crate::executor::{AbortHandle, TaskHandle, spawn, spawn_on_node};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Runtime {
    seed: u64,
    simulation_start_time: Duration,
    fast_forward_time: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: Duration::from_secs(1648339195),
            fast_forward_time: true,
        }
    }
}

impl Runtime {
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn with_simulation_start_time(mut self, simulation_start_time: SystemTime) -> Self {
        self.simulation_start_time = simulation_start_time.duration_since(UNIX_EPOCH).unwrap();
        self
    }

    pub fn with_fast_forward_time(mut self, fast_forward_time: bool) -> Self {
        self.fast_forward_time = fast_forward_time;
        self
    }

    pub fn run<F: Future<Output = ()> + 'static>(&self, f: impl FnOnce() -> F + Send + 'static) {
        self.run_simulation(
            Box::new(|| {
                with_context(|cx| {
                    cx.executor.spawn(NodeId::INIT, f());
                })
            }),
            Box::new(NoopEventHandler),
        );
    }

    pub fn check_determinism<F: Future<Output = ()> + 'static>(
        &self,
        mut future_producer: impl FnMut() -> F + Send,
        iterations: usize,
    ) {
        assert!(iterations > 1);
        let event_handler = RecordingEventHandler::new();
        let mut run_with_events = |events| {
            self.run_simulation(
                Box::new(|| {
                    with_context(|cx| {
                        cx.executor.spawn(NodeId::INIT, future_producer());
                    })
                }),
                events,
            )
        };
        let events = Arc::new(run_with_events(Box::new(event_handler)));
        for _ in 1..iterations {
            run_with_events(Box::new(ValidatingEventHandler::new(events.clone())));
        }
    }

    fn run_simulation(
        &self,
        init_fn: Box<dyn FnOnce() + Send + '_>,
        event_handler: Box<dyn EventHandler>,
    ) -> Vec<Event> {
        // Run the simulation on a new thread to avoid thread local state on this thread interfering
        // with random number generation
        thread::scope(|scope| {
            scope
                .spawn(move || {
                    Context::run(
                        event_handler,
                        self.seed,
                        self.simulation_start_time,
                        init_fn,
                    )
                    .finalize()
                })
                .join()
                .unwrap()
        })
    }
}

pub fn current_node() -> NodeId {
    with_context(|cx| cx.current_node)
}

pub fn create_node() -> NodeId {
    with_context(|cx| cx.new_node())
}

pub fn is_running() -> bool {
    with_context_option(|cx| cx.is_some())
}
