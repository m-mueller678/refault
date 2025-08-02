use std::fmt::Display;
use std::future::Future;
use std::ops::{Add, Sub};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;
use crate::context::CONTEXT;
use crate::event::record_event;

pub fn sleep(duration: Duration) -> TimeFuture {
    TimeFuture::new(duration)
}

pub struct TimeFuture {
    state: Arc<Mutex<TimeFutureState>>,
}

pub(crate) struct TimeFutureState {
    completed: bool,
    waker: Option<Waker>,
}

impl TimeFutureState {
    pub(crate) fn complete(&mut self) {
        self.completed = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl TimeFuture {
    fn new(duration: Duration) -> TimeFuture {
        let state = Arc::new(Mutex::new(
            TimeFutureState {
                completed: false,
                waker: None,
            }
        ));
        let mut context_binding = CONTEXT.lock().unwrap();
        let context = context_binding.as_mut().unwrap();
        context.executor.time_scheduler.lock().unwrap().schedule_future(state.clone(), duration);
        Self {
            state,
        }
    }
}

impl Future for TimeFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();
        if state.completed {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

pub(crate) trait TimeScheduler {
    fn schedule_future(&mut self, future_state: Arc<Mutex<TimeFutureState>>, duration: Duration);
    fn wait_until_next_future_ready(&mut self);
    fn elapsed(&self) -> Duration;
}

pub(crate) struct FastForwardTimeScheduler {
    upcoming_events: Vec<(Duration, Arc<Mutex<TimeFutureState>>)>,
    elapsed_time: Duration,
}

impl FastForwardTimeScheduler {
    pub(crate) fn new() -> Self {
       Self {
           upcoming_events: vec![],
           elapsed_time: Duration::from_secs(0),
       }
    }
}

impl TimeScheduler for FastForwardTimeScheduler {
    fn schedule_future(&mut self, future_state: Arc<Mutex<TimeFutureState>>, duration: Duration) {
        self.upcoming_events.push((self.elapsed_time.add(duration), future_state));
    }

    fn wait_until_next_future_ready(&mut self) {
        if self.upcoming_events.is_empty() {
            return;
        }

        self.upcoming_events.sort_by(|first, second| {
            let (time1, _) = first;
            let (time2, _) = second;
            time1.cmp(time2)
        });
        let (next_event_duration, _) = self.upcoming_events[0];
        record_event(TimeAdvancedEvent::new(next_event_duration.sub(self.elapsed_time)));
        self.elapsed_time = next_event_duration.clone();
        while self.upcoming_events.len() > 0 {
            if self.upcoming_events[0].0 != next_event_duration {
                break;
            }
            let (_, future) =  self.upcoming_events.remove(0);
            future.lock().unwrap().complete();
        }
    }
    
    fn elapsed(&self) -> Duration {
        self.elapsed_time.clone()
    }
}

pub(crate) struct RealisticTimeScheduler {
    upcoming_events: Vec<(Duration, Arc<Mutex<TimeFutureState>>)>,
    elapsed_time: Duration,
}

impl RealisticTimeScheduler {
    pub(crate) fn new() -> Self {
        Self {
            upcoming_events: vec![],
            elapsed_time: Duration::from_secs(0),
        }
    }
}

impl TimeScheduler for RealisticTimeScheduler {
    fn schedule_future(&mut self, future_state: Arc<Mutex<TimeFutureState>>, duration: Duration) {
        self.upcoming_events.push((self.elapsed_time.add(duration), future_state));
    }

    fn wait_until_next_future_ready(&mut self) {
        if self.upcoming_events.is_empty() {
            return;
        }

        self.upcoming_events.sort_by(|first, second| {
            let (time1, _) = first;
            let (time2, _) = second;
            time1.cmp(time2)
        });
        let (next_event_duration, _) = self.upcoming_events[0];
        let sleep_duration = next_event_duration.sub(self.elapsed_time);
        record_event(TimeAdvancedEvent::new(sleep_duration));
        thread::sleep(sleep_duration);
        self.elapsed_time = next_event_duration.clone();
        while self.upcoming_events.len() > 0 {
            if self.upcoming_events[0].0 != next_event_duration {
                break;
            }
            let (_, future) =  self.upcoming_events.remove(0);
            future.lock().unwrap().complete();
        }
    }

    fn elapsed(&self) -> Duration {
        self.elapsed_time.clone()
    }
}

struct TimeAdvancedEvent {
    duration: Duration,
}

impl TimeAdvancedEvent {
    fn new(duration: Duration) -> Box<Self> {
        Box::new(Self {
            duration,
        })
    }
}

impl Display for TimeAdvancedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimeAdvancedEvent{{duration: {:?}}}", self.duration)
    }
}
