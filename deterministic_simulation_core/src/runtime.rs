use crate::context::{Context, CONTEXT};
#[cfg(feature = "log_events")]
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
use crate::executor::{Executor, Task};
use crate::network::{DefaultNetwork, Network};
use crate::node::{reset_nodes, NodeIdSupplier};
use rand_2::SeedableRng;
use rand_chacha_2::ChaCha12Rng;
use std::sync::Arc;

pub struct Runtime {
    pub seed: u64,
    pub simulation_start_time: u64,
    pub network: Arc<dyn Network + Send + Sync + 'static>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: 1750363615882u64,
            network: Arc::new(DefaultNetwork {}),
        }
    }
}

impl Runtime {
    pub fn run(&self, future: impl Future<Output = ()> + Send + Sync + 'static) {
        let options = ExecutionOptions {
            fast_forward_time: false,
            ..Default::default()
        };
        self.run_simulation(future, options);
    }

    pub fn simulate(&self, future: impl Future<Output = ()> + Send + Sync + 'static) {
        let options = ExecutionOptions::default();
        self.run_simulation(future, options);
    }

    #[allow(unused_variables)] // The function panics if event logging is not enabled. In that case, some variables are unused
    pub fn record_events(
        &self,
        future: impl Future<Output = ()> + Send + Sync + 'static,
    ) -> Vec<String> {
        #[cfg(not(feature = "log_events"))]
        panic!(
            "Cannot validate events when event logging is disabled. Please enable the 'log_events' feature."
        );

        #[cfg(feature = "log_events")]
        {
            let event_handler = RecordingEventHandler::new();
            let options = ExecutionOptions {
                event_handler: Box::new(event_handler.clone()),
                ..Default::default()
            };

            self.run_simulation(future, options);

            event_handler.recorded_events.lock().unwrap().clone()
        }
    }

    #[allow(unused_variables)] // The function panics if event logging is not enabled. In that case, some variables are unused
    pub fn validate(
        &self,
        future: impl Future<Output = ()> + Send + Sync + 'static,
        events: Vec<String>,
    ) {
        #[cfg(not(feature = "log_events"))]
        panic!(
            "Cannot validate events when event logging is disabled. Please enable the 'log_events' feature."
        );

        #[cfg(feature = "log_events")]
        {
            let events_size = events.len();
            let event_handler = ValidatingEventHandler::new(events);
            let next_event_index = event_handler.next_event_index.clone();
            let options = ExecutionOptions {
                event_handler: Box::new(event_handler),
                ..Default::default()
            };

            self.run_simulation(future, options);

            if *next_event_index.lock().unwrap() != events_size {
                panic!(
                    "Non-Determinism detected: Expected {} events but only got {}",
                    events_size,
                    *next_event_index.lock().unwrap()
                );
            }
        }
    }

    #[allow(unused_variables)] // The function panics if event logging is not enabled. In that case, some variables are unused
    pub fn check_determinism<T: Future<Output = ()> + Send + Sync + 'static>(
        &self,
        future_producer: fn() -> T,
        iterations: usize,
    ) {
        #[cfg(not(feature = "log_events"))]
        panic!(
            "Check determinism when event logging is disabled. Please enable the 'log_events' feature."
        );

        #[cfg(feature = "log_events")]
        {
            if iterations < 2 {
                panic!("Future must be executed at least twice to check determinism!");
            }

            let events = self.record_events(future_producer());

            #[cfg(not(feature = "print_events"))]
            println!("Recorded {} events: {:?}", events.len(), events);

            for _ in 1..iterations {
                self.validate(future_producer(), events.clone());
            }
        }
    }

    fn run_simulation(
        &self,
        future: impl Future<Output = ()> + Send + Sync + 'static,
        options: ExecutionOptions,
    ) {
        let executor = Arc::new(Executor::new(options.fast_forward_time));
        executor.queue(Arc::new(Task::new(future)));

        self.initialize_context(executor.clone(), options);

        executor.run();
        Self::reset();
    }

    #[allow(unused_variables)] // In case event logging is disabled, 'options' is not actually required
    fn initialize_context(&self, executor: Arc<Executor>, options: ExecutionOptions) {
        let mut context_option = CONTEXT.lock().unwrap();
        match *context_option {
            Some(_) => panic!("Cannot create new context: Context already exists."),
            None => {
                *context_option = Some(Context {
                    executor: executor.clone(),
                    node_id_supplier: NodeIdSupplier::new(),
                    current_node: None,
                    network: self.network.clone(),
                    #[cfg(feature = "log_events")]
                    event_handler: options.event_handler,
                    random_generator: ChaCha12Rng::seed_from_u64(self.seed),
                    simulation_start_time: self.simulation_start_time,
                });
            }
        }
    }

    fn reset() {
        *CONTEXT.lock().unwrap() = None;
        reset_nodes();
    }
}

struct ExecutionOptions {
    fast_forward_time: bool,
    #[cfg(feature = "log_events")]
    event_handler: Box<dyn EventHandler + Send>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            fast_forward_time: true,
            #[cfg(feature = "log_events")]
            event_handler: Box::new(NoopEventHandler),
        }
    }
}
