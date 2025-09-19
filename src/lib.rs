#![feature(abort_unwind)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]

//! A deterministic simulation framework for distributed systems using async.
//!
//! Refault takes your existing async Rust systems and runs one or multiple instances of it in a deterministic simulation.
//! Determinism allows you to reliably reproduce test failures.
//!
//! ```
//! # use std::time::*;
//! # use refault::time::sleep;
//! # use refault::sim_builder::SimBuilder;
//! SimBuilder::new().run(||async move{
//!     let t1 = Instant::now();
//!     sleep(Duration::from_nanos(5)).await;
//!     assert_eq!(t1.elapsed().as_nanos(),5);
//! }).unwrap()
//! ```
//!
//! # How it works
//! Refault aims to eliminate sources of non-determinism from your (distributed) system:
//! - Random number generation is intercepted to return deterministic PRNG sequences, including:.
//!   - [getrandom](https://crates.io/crates/getrandom) and [rand](https://crates.io/crates/rand) if configured properly (see below)
//!   - [std] lib [HashMap] seeding
//!   - the `getrandom` function from libc
//! - Timekeeping functions are intercepted to return a simulated time that progresses deterministically:
//!   - [std::time::Instant]
//!   - [std::time::SystemTime]
//!   - the `clock_gettime` function from libc
//! - Custom Async Executor:
//!   - Single threaded deterministic scheduling
//!   - Support for multiple nodes to simulate distributed systems
//!   - Fast forward simulated time when all tasks are sleeping
//!
//! # Utilities
//! Refault ships with a few useful building blocks to help you port your system to the refault simulation.
//!
//! #### Simulated Packet network
//! [Net](net::Net) implements a simulated packet network.
//! Network abnormalities can be simulated using a user provided [SendFunction](net::SendFunction).
//!
//! #### Send type wrappers
//! Many refault types are often not [Send] or [Sync], as synchronization is not needed in the single-threaded runtime.
//! For compatibility with the Rust async ecosystem, provides wrappers in [send_bind] that make the wrapped value [Send] and [Sync] and ensure it is not accidentally moved out of the simulation or between nodes.
//!
//! #### Id generation
//! You can generate unique Ids for use in your simulation code using [id].
//!
//! #### Non-Determinism detection:
//! If non-determinism is accidentally introduced, the cause can be very hard to debug.
//! By default, refault runs your simulation multiple times and compares event traces to detect non-determinism.
//! It will tell you at which point the executions diverged.
//! To speed up tests, you may opt out of this via [with_determinsim_check](sim_builder::SimBuilder::with_determinism_check).
//!
//! #### The [Simulator] trait
//! users can register [Simulators](simulator::Simulator).
//! These are notified about various events, like the creation, stopping, and starting of nodes.
//! They can also be used to hold simulation-scoped global state, for example to simulate some external system that your system interacts with.
//!
//! #### Third Party integration
//! Refault integrates with various third party crates to reduce the amount of code you need to cahnge to get your system to run in the simulation:
//! - [agnostic-lite](https://crates.io/crates/agnostic-lite): An abstraction layer for any async runtimes. Refault implements [RuntimeLite](::agnostic_lite::RuntimeLite).
//! - [tower]: Refault comes with an RPC implementation based on the [tower] abstraction
//! - [serde]: Refault types imlement [Serialize](serde::Serialize) and [Deserialize](serde::Deserialize) where it makes sense
//!
//! # Caveats
//!
//! #### Configure `getrandom`
//! Many crates, most notably [rand](https://crates.io/crates/rand), use [getrandom](https://crates.io/crates/getrandom) to fetch random numbers from the OS.
//! If you are using [rand](https://crates.io/crates/rand) (or [getrandom](https://crates.io/crates/getrandom)), you should explicitly configure the getrandom backend so that refault can intercept it.
//! You can do so by adding this to your [`.cargo/config.toml`](https://doc.rust-lang.org/cargo/reference/config.html):
//! ```toml
//! rustflags = ['--cfg', 'getrandom_backend="linux_getrandom"']
//! ```
//! Refault may work out of the box on your platform if `getrandom` chooses this backend by default, but this may break on other platforms or when `getrandom` updates.
//!
//! #### Avoid Spawning your own threads
//! Refault uses thread local storage to manage its state.
//! Many refault functions will panic if invoked from a different thread.
//! Moreover, multithreading has the potential to introduce all sorts of non-determinsm.
//! If you have a reasonable large number of tests or are fuzz-testing, you can get plenty of parallelism by running one test per thread.
//! If you have only a small number of tests, running each one on a single thread is probably fast enough.
//!
//! #### Avoid Global variables
//! The state of global variables at the begin of a simulation has the potential to introduce non-determinism.
//! Refault cannot ensure that user-defined global variables always start out in the same state when the simulation starts.
//! If you want to keep global state, consider using [Simulators](crate::simulator).
//!
//! Refault runs each simulation in a fresh thread, so using thread local variables might be fine, depending on how you initialize them.
//! Facilities of the standrad library such as [std::thread_local] should work fine.
//! However, platform specific mechanisms may perform initialization before refault sets up the simulation context on the thread, so they might bypass the interception of OS facilities.
//!
//! # Cargo features
//! | name | description |
//! | --- | --- |
//! | `serde` | derive `Serialize` and `Deserialize` on refault types where applicable. |
//! | `tower` | provide RPC functionality via [Net](net::Net) based on [tower] |
//! | `agnostic-lite` | provide a [RuntimeLite](::agnostic_lite::RuntimeLite) implementation |
//! | `emit-tracing` | emit tracing data via [tracing]. This is fairly noisy but sometimes helpful for debugging |
//! | `send-bind` | Provide wrappers for ensuring data is not moved inappropriately out of the simulation or between nodes and to make types `Send` and `Sync` |
//!
use crate::{
    event::EventHandler,
    executor::{Executor, ExecutorQueue},
    send_bind::ThreadAnchor,
    simulator::Simulator,
};
use rand_chacha::ChaCha12Rng;
use scopeguard::guard;
use std::{
    any::TypeId,
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    time::Duration,
};

#[cfg(feature = "agnostic-lite")]
pub mod agnostic_lite;
mod context_install_guard;
mod event;
pub mod executor;
pub mod id;
mod interception;
pub mod net;
pub use node_id::NodeId;
mod node_id;
#[cfg(feature = "send-bind")]
pub mod send_bind;
#[cfg(feature = "send-bind")]
mod send_bind_util;
pub mod sim_builder;
pub mod simulator;
pub mod time;
#[cfg(feature = "tower")]
pub mod tower;

struct SimCx {
    context: RefCell<Option<SimCxl>>,
    queue: RefCell<Option<ExecutorQueue>>,
    once_cell: OnceCell<SimCxu>,
}

struct SimCxl {
    executor: Executor,
    event_handler: Box<dyn EventHandler>,
    simulators_by_type: HashMap<TypeId, usize>,
    simulators: Vec<Option<Box<dyn Simulator>>>,
}

struct SimCxu {
    current_node: Cell<crate::node_id::NodeId>,
    thread_anchor: ThreadAnchor,
    pre_next_global_id: Cell<u64>,
    time: Cell<Duration>,
    rng: RefCell<ChaCha12Rng>,
}

/// Returns true if called from within a simulation.
pub fn is_in_simulation() -> bool {
    SimCx::with(|cx| cx.once_cell.get().is_some())
}

impl SimCx {
    pub fn with<R>(f: impl FnOnce(&SimCx) -> R) -> R {
        thread_local! {
            static CONTEXT: SimCx = const {SimCx{
                context:RefCell::new(None),
                queue:RefCell::new(None),
                once_cell:OnceCell::new(),
            }};
        }

        CONTEXT.with(f)
    }

    fn cxu(&self) -> &SimCxu {
        self.once_cell.get().unwrap()
    }
    fn current_node(&self) -> crate::node_id::NodeId {
        self.cxu().current_node.get()
    }
    fn node_scope<R>(&self, node: crate::node_id::NodeId, f: impl FnOnce() -> R) -> R {
        let cx3 = self.cxu();
        let calling_node = cx3.current_node.get();
        cx3.current_node.set(node);
        let _guard = guard((), |()| cx3.current_node.set(calling_node));
        f()
    }

    fn with_in_node<R>(node: crate::node_id::NodeId, f: impl FnOnce(&SimCx) -> R) -> R {
        Self::with(|cx| cx.node_scope(node, || f(cx)))
    }

    fn with_cx<R>(&self, f: impl FnOnce(&mut SimCxl) -> R) -> R {
        f(self.context.borrow_mut().as_mut().unwrap())
    }
}

impl SimCxl {
    pub fn with<R>(f: impl FnOnce(&mut SimCxl) -> R) -> R {
        SimCx::with(|cx| f(cx.context.borrow_mut().as_mut().unwrap()))
    }
}
