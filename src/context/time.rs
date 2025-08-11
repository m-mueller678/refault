//! Waiting for the passage of simulated time.
//!
//! Unlike real physical time, simulated time does not advance on its own.
//! The runtime drives all tasks as far as possible before advancing time.
//! When there are no more tasks ready to be run, the runtime checks if there are any tasks waiting on a [Sleep] future.
//! If so, time is advanced to the precise instant at which the next task will be awoken.
//! If there are no tasks ready to be run and no tasks waiting on a [Sleep], the simulation ends.
//!
//! Within the simulation, the standard library time functions return simulated time.
//! To control the starting time of the simulation, see [with_simulation_start_time](crate::runtime::Runtime::with_simulation_start_time).

use crate::context::with_context;
use crate::event::{Event, EventHandler};
use pin_arc::{PinRc, PinRcStorage};
use priority_queue::PriorityQueue;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

pin_project_lite::pin_project! {
    /// The future returned by [sleep] and [sleep_until].
    pub struct Sleep {
        #[pin]
        state: PinRcStorage<Cell<TimeFutureState>>,
    }
}

enum TimeFutureState {
    Init(Instant),
    Waiting(Waker),
    Done,
}

impl Sleep {
    fn new(deadline: Instant) -> Self {
        Sleep {
            state: PinRcStorage::new(Cell::new(TimeFutureState::Init(deadline))),
        }
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, fut_cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state_storage = self.project().state;
        let state = state_storage.as_ref().get_pin();
        match state.replace(TimeFutureState::Done) {
            TimeFutureState::Init(instant) => with_context(|cx| {
                if cx.executor.time_scheduler.now >= instant {
                    Poll::Ready(())
                } else {
                    state.set(TimeFutureState::Waiting(fut_cx.waker().clone()));
                    cx.executor
                        .time_scheduler
                        .upcoming_events
                        .push(QueueEntry(state_storage.as_ref().create_handle()), instant);
                    Poll::Pending
                }
            }),
            TimeFutureState::Waiting(mut waker) => {
                waker.clone_from(fut_cx.waker());
                state.set(TimeFutureState::Waiting(waker));
                Poll::Pending
            }
            TimeFutureState::Done => Poll::Ready(()),
        }
    }
}

pub(crate) struct TimeScheduler {
    upcoming_events: PriorityQueue<QueueEntry, Instant>,
    now: Instant,
}

struct QueueEntry(PinRc<Cell<TimeFutureState>>);

impl QueueEntry {
    fn addr(&self) -> usize {
        let ptr: *const Cell<TimeFutureState> = self.0.get_pin().get_ref();
        ptr.addr()
    }
}

impl std::hash::Hash for QueueEntry {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.addr(), state)
    }
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.addr() == other.addr()
    }
}

impl Eq for QueueEntry {}
impl TimeScheduler {
    pub(crate) fn new() -> Self {
        Self {
            upcoming_events: PriorityQueue::new(),
            now: Instant::now(),
        }
    }

    pub(crate) fn wait_until_next_future_ready(
        &mut self,
        time: &Cell<Option<Duration>>,
        event_handler: &mut dyn EventHandler,
    ) -> bool {
        let Some(next) = self.upcoming_events.peek().map(|x| *x.1) else {
            return false;
        };
        let dt = next.duration_since(self.now);
        event_handler.handle_event(Event::TimeAdvanced(dt));
        time.set(Some(time.get().unwrap() + dt));
        self.now = next;
        while let Some(x) = self.upcoming_events.pop_if(|_, t| *t <= self.now) {
            let TimeFutureState::Waiting(waker) = x.0.0.get_pin().replace(TimeFutureState::Done)
            else {
                unreachable!();
            };
            waker.wake();
        }
        true
    }
}

/// Wait until `deadline`.
pub fn sleep_until(deadline: Instant) -> Sleep {
    Sleep::new(deadline)
}

/// Wait for `duration`.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep::new(Instant::now() + duration)
}
