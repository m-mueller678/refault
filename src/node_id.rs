use std::num::NonZeroUsize;

use crate::{
    SimCx, SimCxl,
    event::Event,
    executor::{spawn_task_on_node, stop_node},
    simulator::for_all_simulators,
};

/// A unique identifier for a node within a simulation.
#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeId(pub(crate) NonZeroUsize);

impl NodeId {
    pub const INIT: Self = NodeId::from_index(0);

    /// Create a new node that tasks can be run on.
    ///
    /// Can only be called from within a simulation.
    pub fn create_node() -> NodeId {
        SimCx::with(|cx| {
            let id = cx.with_cx(|cxl| {
                let id = cxl.executor.push_new_node();
                cxl.event_handler.handle_event(Event::NodeSpawned(id));
                id
            });
            cx.node_scope(id, || {
                for_all_simulators(cx, true, |s| {
                    s.create_node();
                });
            });
            id
        })
    }

    #[cfg(feature = "emit-tracing")]
    pub(crate) fn tv(&self) -> impl tracing::Value {
        self.0.get()
    }

    /// Spawn a task on this node and detach it.
    pub fn spawn<F: Future + 'static>(self, future: F) {
        spawn_task_on_node(self, future).detach();
    }

    /// Returns the id of the node this task is running on.
    ///
    /// Can only be called from within a simulation.
    pub fn current() -> Self {
        SimCx::with(|cx| cx.current_node())
    }

    /// Invoke Simulator::stop on all simulators and stop all tasks on this node.
    ///
    /// Attempting to spawn tasks on a stopped node will panic.
    /// Attempting to stop a node that is already stopped does nothing.
    pub fn stop(self) {
        stop_node(self, false);
    }

    /// Iterate over all nodes in the current simulation.
    ///
    /// If new nodes are added while this iterator exists, panics may occur or they may or may not be returned.
    pub fn all() -> impl Iterator<Item = NodeId> {
        (0..SimCxl::with(|cx| cx.executor.node_count())).map(NodeId::from_index)
    }

    pub(crate) const fn from_index(index: usize) -> Self {
        NodeId(NonZeroUsize::new(index + 1).unwrap())
    }

    pub(crate) const fn to_index(self) -> usize {
        self.0.get() - 1
    }
}
