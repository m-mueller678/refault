//! Running simulations.
use futures::never::Never;
use sync_wrapper::SyncWrapper;

use crate::SimCx;
use crate::context_install_guard::ContextInstallGuard;
use crate::event::{Event, HashRecordingEventHandler, HashValidatingEventHandler};
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
use crate::executor::{Executor, spawn, stop_simulation};
use std::any::Any;
use std::panic::resume_unwind;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{mem, thread};

/// Running simulations.
pub struct SimBuilder {
    seed: u64,
    simulation_start_time: Duration,
    determinism_check: DeterminismCheck,
}

/// Specifes the determinism checks performed for a test.
///
/// Can be set via [SimBuilder::with_determinism_check].
pub enum DeterminismCheck {
    /// Run once with no determinism check
    None,
    /// Record the first run and compare all other runs for equality.
    Full {
        /// The total number fo runs performed, including the first one.
        /// Must be at least 2.
        iterations: usize,
    },
    /// Record a hash of the first run and compare all other runs for equality.
    /// This gives less usefull error messages than [Full](Self::Full), but uses less memory.
    Hash { iterations: usize },
}

/// The output of a simulation.
///
/// This contains the output of the main future along with additional info.
#[derive(Debug)]
#[must_use]
pub struct SimulationOutput<T> {
    /// The amount of simmulated time elapsed.
    pub time_elapsed: Duration,
    /// The output of the root future if it completed.
    /// `None` if the simulation ended before it completed.
    /// This can happen for two reasons:
    /// - no more progress could be made (none of the tasks are ready and wake will never be called on them)
    /// - the simulation was stopped via [stop_simulation].
    pub output: Option<T>,
    pub(crate) events: Box<dyn Any + Send>,
}

impl<T> SimulationOutput<T> {
    /// Unwrap the output of the main future.
    ///
    /// This is convenient for verifying that the main future did in fact complete.
    #[track_caller]
    pub fn unwrap(self) -> T {
        self.output
            .expect("simulation root future did not complete")
    }
}

impl Default for SimBuilder {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: Duration::from_secs(1648339195),
            determinism_check: DeterminismCheck::Full { iterations: 2 },
        }
    }
}

impl SimBuilder {
    /// Construct a new builder.
    ///
    /// The builder can be used as is to run a simulation, or have more configuration options set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the seed used for random number generation.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Set the system time at the start of the simulation.
    pub fn with_simulation_start_time(mut self, simulation_start_time: SystemTime) -> Self {
        self.simulation_start_time = simulation_start_time.duration_since(UNIX_EPOCH).unwrap();
        self
    }

    /// Set the determinism check behaviour.
    ///
    /// If you are planning to disable determinism checks to speed up tests, consider making it dependent on an environment variable.
    /// Non-determinism can be very frustrating to debug and you may at first not even notice it is happening, so you should have an easy way to turn these checks back on for selected runs.
    pub fn with_determinism_check(mut self, determinism_check: DeterminismCheck) -> Self {
        self.determinism_check = determinism_check;
        self
    }

    /// Run the simulation.
    ///
    /// Within the simulation, `f` is invoked to create the root future of the simulation, which is then spawned on the runtime's executor.
    /// After `f` completes, the executor starts running.
    /// Both `f` and the future run on the node [`NodeId::INIT`](crate::NodeId::INIT).
    /// The simulation continues until the root future completes or no task can make any more progress.
    pub fn run<F: Future<Output: Send> + 'static>(
        &self,
        mut f: impl FnMut() -> F + Send,
    ) -> SimulationOutput<F::Output> {
        let mut run_with_events = |events| {
            let ret = Arc::new(OnceLock::new());
            let ret2 = ret.clone();
            let output = self.run_simulation(
                Box::new(|| {
                    let fut = f();
                    spawn(async move {
                        let result = fut.await;
                        ret2.set(SyncWrapper::new(result)).ok().unwrap();
                        stop_simulation();
                    })
                    .detach();
                }),
                events,
            );
            SimulationOutput {
                time_elapsed: output.time_elapsed - self.simulation_start_time,
                output: Arc::into_inner(ret)
                    .unwrap()
                    .into_inner()
                    .map(SyncWrapper::into_inner),
                events: output.events,
            }
        };
        match self.determinism_check {
            DeterminismCheck::None => run_with_events(Box::new(NoopEventHandler)),
            DeterminismCheck::Full { iterations } => {
                assert!(iterations > 1);
                let mut output = run_with_events(Box::new(RecordingEventHandler::new()));
                let events = Arc::new(
                    *mem::replace(&mut output.events, Box::new(()))
                        .downcast::<Vec<Event>>()
                        .unwrap(),
                );
                for _ in 1..iterations {
                    let _ = run_with_events(Box::new(ValidatingEventHandler::new(events.clone())));
                }
                output
            }
            DeterminismCheck::Hash { iterations } => {
                assert!(iterations > 1);
                let mut output = run_with_events(Box::<HashRecordingEventHandler>::default());
                let event_hash = *mem::replace(&mut output.events, Box::new(()))
                    .downcast::<u64>()
                    .unwrap();
                for _ in 1..iterations {
                    let _ = run_with_events(Box::new(HashValidatingEventHandler::new(event_hash)));
                }
                output
            }
        }
    }

    #[doc(hidden)]
    // This is used to test refault
    pub fn new_test() -> Self {
        static ITERATIONS: LazyLock<usize> = LazyLock::new(|| {
            let x = std::env::var("REFAULT_ITERATIONS")
                .ok()
                .map(|x| x.parse().unwrap())
                .unwrap_or(3);
            println!("running {x} iterations of all tests");
            x
        });
        Self::default().with_determinism_check(DeterminismCheck::Full {
            iterations: *ITERATIONS,
        })
    }

    fn run_simulation(
        &self,
        init_fn: Box<dyn FnOnce() + Send + '_>,
        event_handler: Box<dyn EventHandler>,
    ) -> SimulationOutput<Never> {
        // Run the simulation on a new thread to avoid thread local state on this thread interfering
        // with random number generation
        thread::scope(|scope| {
            let result = thread::Builder::new()
                .name("simulation".to_owned())
                .spawn_scoped(scope, move || {
                    let mut context_guard = ContextInstallGuard::new(
                        event_handler,
                        self.seed,
                        self.simulation_start_time,
                    );
                    init_fn();
                    SimCx::with(|cx| {
                        Executor::run_current_context(cx);
                    });
                    context_guard.destroy(true).unwrap()
                })
                .unwrap()
                .join();
            match result {
                Ok(x) => x,
                Err(e) => resume_unwind(e),
            }
        })
    }
}
