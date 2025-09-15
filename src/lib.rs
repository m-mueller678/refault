#![feature(abort_unwind)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]
//! Many functions within this crate should only be called from within a simulation and will panic otherwise.
//! [is_in_simulation] may be used to check if inside the simulation.

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
pub mod runtime;
#[cfg(feature = "send-bind")]
pub mod send_bind;
#[cfg(feature = "send-bind")]
mod send_bind_util;
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
