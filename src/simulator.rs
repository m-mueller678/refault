//! Simulation-scoped singletons.
//!
//! Within a simulation global objects implementing the [Simulator] trait may be registered and accessed.
//! These can be used to simulate some external system the program needs to interact with.
//! For instance, the [PacketNetwork](crate::packet_network::PacketNetwork) simulator contains global state to implement a virtual network between nodes.
//!
//! A simulator can be registered using [add_simulator] and later accessed using [with_simulator].
//! At the end of a simulation, all associated simulators will be destroyed.
//! There is at most one object for each simulator type in a simulation.

use std::{
    any::{Any, TypeId, type_name},
    mem,
};

use crate::context::{Context2, NodeId, with_context};

/// A simulation-scoped singleton.
///
/// Simulators are notified by the runtime about various events.
/// The callbacks are invoked from the context of the affected node, not the node triggering the event.
/// It is entirely up to the implementor how they react to these events.
/// The default implementations do nothing.
pub trait Simulator: Any {
    /// A new node was created via [NodeId::create_node].
    /// This is not invoked for the first node, which is created by the runtime to run the function passed to [run](crate::runtime::Runtime::run) and the future it returns.
    fn create_node(&mut self) {}
    /// The current node is being shutdown.
    /// This is called before cancelling all tasks.
    fn stop_node(&mut self) {}
    /// The current node was started after previously having been shutdown.
    /// This is not called when the node starts running after creation.
    fn start_node(&mut self) {}
}

/// Add a simulator to the simulation.
///
/// Panics if a simulator with of type has already been added.
pub fn add_simulator<S: Simulator>(simulator: S) {
    let simulator = Some(Box::new(simulator) as Box<_>);
    with_context(|cx| {
        assert!(
            cx.simulators_by_type
                .insert(TypeId::of::<S>(), cx.simulators.len())
                .is_none()
        );
        cx.simulators.push(simulator);
    })
}

/// Call `f` with a mutable reference to the simulator if it exists or `None` otherwise.
fn with_simulator_option<S: Simulator, R>(f: impl FnOnce(Option<&mut S>) -> R) -> R {
    if let Some((index, mut simulator)) = with_context(|cx| {
        let index = *cx.simulators_by_type.get(&TypeId::of::<S>())?;

        let simulator = cx.simulators[index]
            .take()
            .unwrap_or_else(|| panic!("simulator already mutably borrowed: {}", type_name::<S>()));
        Some((index, simulator))
    }) {
        let ret = f(Some(
            (&mut *simulator as &mut dyn Any).downcast_mut().unwrap(),
        ));
        with_context(|cx| {
            assert!(cx.simulators[index].replace(simulator).is_none());
        });
        ret
    } else {
        f(None)
    }
}

/// Call `f` with a mutable reference to the simulator.
///
/// Panics if no simulator of this type has been added.
pub fn with_simulator<S: Simulator, R>(f: impl FnOnce(&mut S) -> R) -> R {
    with_simulator_option(|mut x| {
        f(x.as_mut()
            .unwrap_or_else(|| panic!("simulator does not exist: {}", type_name::<S>())))
    })
}

pub(crate) fn for_all_simulators(forward: bool, mut f: impl FnMut(&mut dyn Simulator)) {
    let len = with_context(|cx| cx.simulators.len());
    let mut simulator = None;
    Context2::with(|cx2| {
        for i in 0..len {
            let index = if forward { i } else { len - 1 - i };
            let swap = |s: &mut Option<_>| {
                mem::swap(
                    &mut cx2.context.borrow_mut().as_mut().unwrap().simulators[index],
                    s,
                )
            };
            swap(&mut simulator);
            f(&mut **simulator
                .as_mut()
                .expect("simulator already mutably borrowed"));
            swap(&mut simulator);
        }
        assert_eq!(
            cx2.context.borrow_mut().as_mut().unwrap().simulators.len(),
            len
        );
    });
}

/// A node-scoped singleton.
///
/// This is a node-scoped version of [Simulator].
/// Adding a `NodeSimulator` to a simulation is like adding a `Simulator` that holds one `NodeSimulator` object for each node.
pub trait NodeSimulator: 'static {
    fn stop_node(&mut self);
    fn start_node(&mut self);
    fn new() -> Self;
}

struct NodeSimSim<S: NodeSimulator> {
    simulators: Vec<S>,
}

impl<S: NodeSimulator> Simulator for NodeSimSim<S> {
    fn create_node(&mut self) {
        self.simulators.push(S::new())
    }

    fn stop_node(&mut self) {
        self.simulators[NodeId::current().0].stop_node();
    }

    fn start_node(&mut self) {
        self.simulators[NodeId::current().0].start_node();
    }
}

/// Add a new node simulator.
pub fn add_node_simulator<S: NodeSimulator>() {
    debug_assert_eq!(NodeId::current(), NodeId::INIT);
    add_simulator(NodeSimSim {
        simulators: vec![S::new()],
    });
}

/// Call `f` with a mutable reference to the node-simulator.
///
/// Panics if no node-simulator of this type has been added.
pub fn with_node_simulator<S: NodeSimulator, R>(f: impl FnOnce(&mut S) -> R) -> R {
    with_simulator::<NodeSimSim<S>, _>(|s| (f(&mut s.simulators[NodeId::current().0])))
}
