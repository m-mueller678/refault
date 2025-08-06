#[cfg(feature = "log_events")]
use crate::context::{Context, CONTEXT};
#[cfg(feature = "log_events")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "log_events")]
type Event = dyn ToString;

#[cfg(feature = "log_events")]
pub fn record_event(event: Box<Event>) {
    #[cfg(feature = "print_events")]
    println!("{}", event.to_string());

    let mut mutex_guard = CONTEXT.lock().unwrap();
    let context: &mut Context = mutex_guard.as_mut().unwrap();
    context.event_handler.handle_event(event);
}

#[cfg(not(feature = "log_events"))]
pub fn record_event(_: Box<dyn ToString>) {
    // Do nothing
}

#[cfg(feature = "log_events")]
pub(crate) trait EventHandler {
    #[allow(dead_code)] // Not required when event recording is disabled
    fn handle_event(&self, event: Box<Event>);
}

#[cfg(feature = "log_events")]
pub(crate) struct NoopEventHandler;

#[cfg(feature = "log_events")]
impl EventHandler for NoopEventHandler {
    fn handle_event(&self, _: Box<Event>) {}
}

#[cfg(feature = "log_events")]
#[derive(Clone)]
pub(crate) struct RecordingEventHandler {
    pub(crate) recorded_events: Arc<Mutex<Vec<String>>>,
}

#[cfg(feature = "log_events")]
impl RecordingEventHandler {
    pub(crate) fn new() -> Self {
        Self {
            recorded_events: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[cfg(feature = "log_events")]
impl EventHandler for RecordingEventHandler {
    fn handle_event(&self, event: Box<Event>) {
        self.recorded_events.lock().unwrap().push(event.to_string());
    }
}

#[cfg(feature = "log_events")]
pub(crate) struct ValidatingEventHandler {
    events_to_validate: Vec<String>,
    pub(crate) next_event_index: Arc<Mutex<usize>>,
}

#[cfg(feature = "log_events")]
impl ValidatingEventHandler {
    pub(crate) fn new(previous_events: Vec<String>) -> Self {
        Self {
            events_to_validate: previous_events,
            next_event_index: Arc::new(Mutex::new(0)),
        }
    }
}

#[cfg(feature = "log_events")]
impl EventHandler for ValidatingEventHandler {
    fn handle_event(&self, event: Box<Event>) {
        let mut index_mutex_guard = self.next_event_index.lock().unwrap();
        let current_event = event.to_string();
        if *index_mutex_guard >= self.events_to_validate.len() {
            panic!(
                "Non-Determinism detected: Expected no further events, but got '{}'",
                current_event
            );
        }
        if current_event != self.events_to_validate[*index_mutex_guard] {
            panic!(
                "Non-Determinism detected: Validation failed for event {}: Expected '{}', but got '{}'",
                *index_mutex_guard, self.events_to_validate[*index_mutex_guard], current_event
            );
        }
        *index_mutex_guard += 1;
    }
}
