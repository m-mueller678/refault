use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use reduced_rand::SeedableRng;
use reduced_rand_chacha::ChaCha12Rng;
use crate::context::{Context, CONTEXT};
use crate::network::DefaultNetwork;
use crate::node::{NodeIdSupplier, NODES};
use crate::task::{Executor, Task};

type Event = dyn ToString;

#[cfg(feature = "log_events")]
const EVENT_LOGGING_ENABLED: bool = true;

#[cfg(not(feature = "log_events"))]
const EVENT_LOGGING_ENABLED: bool = false;

#[cfg(feature = "log_events")]
pub fn record_event(event: Box<Event>) {
    #[cfg(feature = "print_events")]
    println!("{}", event.to_string());

    let mut binding = CONTEXT.lock().unwrap();
    let context: &mut Context = binding.as_mut().unwrap();
    context.event_handler.handle_event(event);
}

#[cfg(not(feature = "log_events"))]
pub fn record_event(_: Box<dyn ToString>) {
}

pub(crate) trait EventHandler {
    #[allow(dead_code)]
    fn handle_event(&self, event: Box<Event>);
}

pub(crate) struct NoopEventHandler;
impl EventHandler for NoopEventHandler {
    fn handle_event(&self, _: Box<Event>) {}
}

#[derive(Clone)]
struct RecordingEventHandler {
    recorded_events: Arc<Mutex<Vec<String>>>,
}

impl RecordingEventHandler {
    fn new() -> Self {
        Self {
            recorded_events: Arc::new(Mutex::new(vec![])),
        }
    }
}

impl EventHandler for RecordingEventHandler {
    fn handle_event(&self, event: Box<Event>) {
        self.recorded_events.lock().unwrap().push(event.to_string());
    }
}

struct ValidatingEventHandler {
    previous_events: Vec<String>,
    next_event_index: Arc<Mutex<usize>>,
}

impl ValidatingEventHandler {
    fn new(previous_events: Vec<String>) -> Self {
        Self {
            previous_events,
            next_event_index: Arc::new(Mutex::new(0)),
        }
    }
}

impl EventHandler for ValidatingEventHandler {
    fn handle_event(&self, event: Box<Event>) {
        let mut index_binding = self.next_event_index.lock().unwrap();
        let current_event = event.to_string();
        if *index_binding >= self.previous_events.len() {
            panic!("Non-Determinism detected: Expected no further events, but got '{}'", current_event);
        }
        if current_event != self.previous_events[*index_binding] {
            panic!(
                "Non-Determinism detected: Validation failed for event {}: Expected '{}', but got '{}'",
                *index_binding, self.previous_events[*index_binding], current_event
            );
        }
        *index_binding += 1;
    }
}

#[allow(dead_code)]
pub(crate) fn validate(future: impl Future<Output = ()> + Send + Sync + 'static, events: Vec<String>) {
    if !EVENT_LOGGING_ENABLED {
        panic!("Cannot validate events when event logging is disabled. Please enable the 'log_events' feature.");
    }
    
    let executor = Arc::new(Executor::new(true));
    
    let events_size = events.len();
    let event_handler = ValidatingEventHandler::new(events);
    let next_event_index = event_handler.next_event_index.clone();

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
                    event_handler: Box::new(event_handler),
                    random_generator: ChaCha12Rng::seed_from_u64(0u64),
                });
            }
        }
    }

    executor.run();
    *CONTEXT.lock().unwrap() = None;
    *NODES.lock().unwrap() = Vec::new();
    
    if *next_event_index.lock().unwrap() != events_size {
        panic!("Non-Determinism detected: Expected {} events but only got {}", events_size, *next_event_index.lock().unwrap());
    }
}

#[allow(dead_code)]
pub(crate) fn record_events(future: impl Future<Output = ()> + Send + Sync + 'static) -> Vec<String> {
    if !EVENT_LOGGING_ENABLED {
        panic!("Cannot record events when event logging is disabled. Please enable the 'log_events' feature.");
    }
    
    let executor = Arc::new(Executor::new(true));
    
    let event_handler = RecordingEventHandler::new();

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
                    event_handler: Box::new(event_handler.clone()),
                    random_generator: ChaCha12Rng::seed_from_u64(0u64),
                });
            }
        }
    }

    executor.run();
    *CONTEXT.lock().unwrap() = None;
    *NODES.lock().unwrap() = Vec::new();
    
    let binding = event_handler.recorded_events.lock().unwrap();
    binding.clone()
}

#[allow(dead_code)]
pub fn check_determinism(future_producer: fn() -> Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>, iterations: usize) {
    if !EVENT_LOGGING_ENABLED {
        panic!("Check determinism when event logging is disabled. Please enable the 'log_events' feature.");
    }
    
    if iterations < 2 {
        panic!("Future must be executed at least twice to check determinism!");
    }
    
    let events = record_events(future_producer());

    #[cfg(feature = "print_events")]
    println!("Recorded {} events: {:?}", events.len(), events);

    for _ in 1..iterations {
        validate(future_producer(), events.clone());
    }
}
