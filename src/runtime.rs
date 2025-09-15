//! Controlling simulations, tasks and nodes.
use crate::SimCx;
use crate::context_install_guard::ContextInstallGuard;
use crate::event::Event;
use crate::event::{EventHandler, NoopEventHandler, RecordingEventHandler, ValidatingEventHandler};
use crate::executor::Executor;
use crate::node_id::NodeId;
use std::panic::resume_unwind;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The simulation runtime.
///
/// A Runtime object is used to configure and start simulations.
pub struct Runtime {
    seed: u64,
    simulation_start_time: Duration,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            seed: 0,
            simulation_start_time: Duration::from_secs(1648339195),
        }
    }
}

impl Runtime {
    /// Construct a new runtime with the default configuration.
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

    /// Run a simulation.
    /// Within the simulation, `f` is invoked to create a future, which is then spawned on the runtime's executor.
    /// After `f` completes, the executor starts running.
    /// The simulation continues until no task can make any more progress.
    // TODO return number of remaining tasks
    pub fn run<F: Future<Output = ()> + 'static>(&self, f: impl FnOnce() -> F + Send + 'static) {
        self.run_simulation(
            Box::new(|| {
                NodeId::INIT.spawn(f());
            }),
            Box::new(NoopEventHandler),
        );
    }

    /// Runs a simulation repeatedly to check if it is deterministic.
    /// Various events are recorded during the execution.
    /// If the sequence of events differs between runs, a panic is raised.
    /// See [run](Self::run) for details how the simulation is run.
    pub fn check_determinism<F: Future<Output = ()> + 'static>(
        &self,
        iterations: usize,
        mut f: impl FnMut() -> F + Send,
    ) {
        assert!(iterations > 1);
        let event_handler = RecordingEventHandler::new();
        let mut run_with_events = |events| {
            self.run_simulation(
                Box::new(|| {
                    NodeId::INIT.spawn(f());
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
                    context_guard.destroy().unwrap().finalize()
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
