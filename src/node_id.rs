use std::num::NonZeroUsize;

use crate::{
    Context2,
    event::Event,
    executor::{Executor, spawn_task_on_node},
    simulator::for_all_simulators,
    with_context,
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
        Context2::with(|cx2| {
            let id = cx2.with_cx(|cx| {
                let id = cx.executor.push_new_node();
                cx.event_handler.handle_event(Event::NodeSpawned(id));
                id
            });
            cx2.node_scope(id, || {
                for_all_simulators(cx2, true, |s| {
                    s.create_node();
                });
            });
            id
        })
    }

    /// Spawn a task on this node and detach it.
    pub fn spawn<F: Future + 'static>(self, future: F) {
        spawn_task_on_node(self, future).detach();
    }

    /// Returns the id of the node this task is running on.
    ///
    /// Can only be called from within a simulation.
    pub fn current() -> Self {
        Context2::with(|cx2| {
            if cx2.time.get().is_some() {
                Some(cx2.current_node())
            } else {
                None
            }
        })
        .expect("not inside a simulation")
    }

    /// Invoke Simulator::stop on all simulators and stop all tasks on this node.
    ///
    /// Attempting to spawn tasks on a stopped node will panic.
    /// Attempting to stop a node that is already stopped does nothing.
    pub fn stop(self) {
        Executor::stop_node(self, false);
    }

    /// Iterate over all nodes in the current simulation.
    ///
    /// If new nodes are added while this iterator exists, panics may occur or they may or may not be returned.
    pub fn all() -> impl Iterator<Item = NodeId> {
        (0..with_context(|cx| cx.executor.node_count())).map(NodeId::from_index)
    }

    pub(crate) const fn from_index(index: usize) -> Self {
        NodeId(NonZeroUsize::new(index + 1).unwrap())
    }

    pub(crate) const fn to_index(self) -> usize {
        self.0.get() - 1
    }
}
