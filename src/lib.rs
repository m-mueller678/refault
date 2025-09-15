#![feature(abort_unwind)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]
//! Many functions within this crate should only be called from within a simulation and will panic otherwise.
//! [NodeId::try_current](runtime::NodeId::try_current) may be used to check if inside the simulation.

use crate::{
    event::EventHandler,
    executor::{Executor, ExecutorQueue, NodeId},
    send_bind::ThreadAnchor,
    simulator::Simulator,
};
use rand_chacha::ChaCha12Rng;
use scopeguard::guard;
use std::{
    any::TypeId,
    cell::{Cell, RefCell},
    collections::HashMap,
    time::Duration,
};

#[cfg(feature = "agnostic-lite")]
pub mod agnostic_lite_runtime;
mod context_install_guard;
mod event;
pub mod executor;
pub mod id;
mod interception;
pub mod net;
pub mod runtime;
#[cfg(feature = "send-bind")]
pub mod send_bind;
#[cfg(feature = "send-bind")]
mod send_bind_util;
pub mod simulator;
pub mod time;
#[cfg(feature = "tower")]
pub mod tower;

thread_local! {
    static CONTEXT: Context2 = const {Context2{
        context:RefCell::new(None),
        rng:RefCell::new(None),
        time:Cell::new(None),
        queue:RefCell::new(None),
        pre_next_global_id:Cell::new(0),
        current_node:Cell::new(NodeId::INIT),
        thread_anchor: Cell::new(None),
    }};
}

fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    Context2::with(|c| f(&mut c.context.borrow_mut()))
}

fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}

type SimulatorRc = Option<Box<dyn Simulator>>;

struct Context {
    executor: Executor,
    event_handler: Box<dyn EventHandler>,
    simulators_by_type: HashMap<TypeId, usize>,
    simulators: Vec<SimulatorRc>,
}

struct Context2 {
    context: RefCell<Option<Context>>,
    rng: RefCell<Option<ChaCha12Rng>>,
    time: Cell<Option<Duration>>,
    queue: RefCell<Option<ExecutorQueue>>,
    current_node: Cell<NodeId>,
    thread_anchor: Cell<Option<ThreadAnchor>>,
    pre_next_global_id: Cell<u64>,
}

pub fn is_in_simulation() -> bool {
    Context2::with(|cx| cx.time.get().is_some())
}

impl Context2 {
    pub fn with<R>(f: impl FnOnce(&Context2) -> R) -> R {
        CONTEXT.with(f)
    }

    pub fn thread_anchor(&self) -> Option<ThreadAnchor> {
        self.thread_anchor.get()
    }

    pub fn node_scope<R>(&self, node: NodeId, f: impl FnOnce() -> R) -> R {
        let calling_node = self.current_node.get();
        self.current_node.set(node);
        let _guard = guard((), |()| self.current_node.set(calling_node));
        f()
    }

    pub fn with_in_node<R>(node: NodeId, f: impl FnOnce(&Context2) -> R) -> R {
        Self::with(|cx| cx.node_scope(node, || f(cx)))
    }

    pub fn current_node(&self) -> NodeId {
        self.current_node.get()
    }

    pub fn with_cx<R>(&self, f: impl FnOnce(&mut Context) -> R) -> R {
        f(self.context.borrow_mut().as_mut().unwrap())
    }
}
