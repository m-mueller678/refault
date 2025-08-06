use crate::{
    context::{Context, with_context},
    node::NodeId,
};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Event {
    FuturePolledEvent,
    TimeAdvancedEvent(Duration),
    NodeSpawnedEvent { node_id: NodeId },
    TaskSpawnedEvent,
}

pub fn record_event(event: Event) {
    with_context(|context| {
        context.as_mut().unwrap().event_handler.handle_event(event);
    })
}

pub(crate) trait EventHandler {
    fn handle_event(&self, event: Event);
}

pub(crate) struct NoopEventHandler;

impl EventHandler for NoopEventHandler {
    fn handle_event(&self, _: Event) {}
}

#[derive(Clone)]
pub(crate) struct RecordingEventHandler {
    pub(crate) recorded_events: Arc<Mutex<Vec<Event>>>,
}

impl RecordingEventHandler {
    pub(crate) fn new() -> Self {
        Self {
            recorded_events: Arc::new(Mutex::new(vec![])),
        }
    }
}

impl EventHandler for RecordingEventHandler {
    fn handle_event(&self, event: Event) {
        self.recorded_events.lock().unwrap().push(event);
    }
}

pub(crate) struct ValidatingEventHandler {
    events_to_validate: Vec<Event>,
    pub(crate) next_event_index: Arc<Mutex<usize>>,
}

impl ValidatingEventHandler {
    pub(crate) fn new(previous_events: Vec<Event>) -> Self {
        Self {
            events_to_validate: previous_events,
            next_event_index: Arc::new(Mutex::new(0)),
        }
    }
}

impl EventHandler for ValidatingEventHandler {
    fn handle_event(&self, event: Event) {
        let mut index_mutex_guard = self.next_event_index.lock().unwrap();
        let current_event = event;
        if *index_mutex_guard >= self.events_to_validate.len() {
            panic!(
                "Non-Determinism detected: Expected no further events, but got '{current_event:?}'",
            );
        }
        let index = *index_mutex_guard;
        let expected = &self.events_to_validate[index];
        if current_event != *expected {
            panic!(
                "Non-Determinism detected: Validation failed for event {index}: Expected '{expected:?}', but got '{current_event:?}'",
            );
        }
        *index_mutex_guard += 1;
    }
}
