use crate::context::{NodeId, with_context};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Event {
    FuturePolledEvent,
    TimeAdvancedEvent(Duration),
    NodeSpawnedEvent { node_id: NodeId },
    TaskSpawnedEvent,
}

pub fn record_event(event: Event) {
    with_context(|context| {
        context.event_handler.handle_event(event);
    })
}

pub(crate) trait EventHandler: Send {
    fn handle_event(&mut self, event: Event);
    fn finalize(self: Box<Self>) -> Vec<Event>;
}

pub(crate) struct NoopEventHandler;

impl EventHandler for NoopEventHandler {
    fn handle_event(&mut self, _: Event) {}

    fn finalize(self: Box<Self>) -> Vec<Event> {
        Vec::new()
    }
}

#[derive(Clone)]
pub(crate) struct RecordingEventHandler {
    pub(crate) recorded_events: Vec<Event>,
}

impl RecordingEventHandler {
    pub(crate) fn new() -> Self {
        Self {
            recorded_events: Default::default(),
        }
    }
}

impl EventHandler for RecordingEventHandler {
    fn handle_event(&mut self, event: Event) {
        self.recorded_events.push(event);
    }

    fn finalize(self: Box<Self>) -> Vec<Event> {
        self.recorded_events
    }
}

pub(crate) struct ValidatingEventHandler {
    events_to_validate: Arc<Vec<Event>>,
    pub(crate) next_event_index: usize,
}

impl ValidatingEventHandler {
    pub(crate) fn new(previous_events: Arc<Vec<Event>>) -> Self {
        Self {
            events_to_validate: previous_events,
            next_event_index: 0,
        }
    }
}

impl EventHandler for ValidatingEventHandler {
    fn handle_event(&mut self, event: Event) {
        let current_event = event;
        if self.next_event_index >= self.events_to_validate.len() {
            panic!(
                "Non-Determinism detected: Expected no further events, but got '{current_event:?}'",
            );
        }
        let expected = &self.events_to_validate[self.next_event_index];
        if current_event != *expected {
            panic!(
                "Non-Determinism detected: Validation failed for event {index}: Expected '{expected:?}', but got '{current_event:?}'",
                index = self.next_event_index
            );
        }
        self.next_event_index += 1;
    }

    fn finalize(self: Box<Self>) -> Vec<Event> {
        let index = self.next_event_index;
        let len = self.events_to_validate.len();
        if index != len {
            panic!("Non-Determinism detected: Expected {len} events but only got {index}",);
        }
        Vec::new()
    }
}
