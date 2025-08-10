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
    cell::RefCell,
    rc::Rc,
};

use crate::context::{NodeId, with_context};

/// A simulation-scoped singleton.
///
/// Simulators are notified by the runtime about various events.
/// It is entirely up to the implementor how they react to these events.
pub trait Simulator: Any {
    /// A new node was created via [NodeId::create_node].
    /// There may already be multiple nodes in existence at the time a simulator is created.
    /// It will not be notified about these nodes.
    /// For example, this is not invoked for the first node, which is created by the runtime to run the function passed to [run](crate::runtime::Runtime::run) and the future it returns.
    #[allow(unused_variables)]
    fn node_created(&mut self, id: NodeId) {}
}

/// Add a simulator to the simulation.
///
/// Panics if a simulator with of type has already been added.
pub fn add_simulator<S: Simulator>(s: S) {
    with_context(|cx| {
        assert!(
            cx.simulators
                .insert(TypeId::of::<S>(), Rc::new(RefCell::new(s)))
                .is_none()
        );
    })
}

/// Call `f` with a mutable reference to the simulator if it exists or `None` otherwise.
pub fn with_simulator_option<S: Simulator, R>(f: impl FnOnce(Option<&mut S>) -> R) -> R {
    let simulator = with_context(|cx| cx.simulators.get(&TypeId::of::<S>()).cloned());
    let mut simulator = simulator.as_ref().map(|x| x.borrow_mut());
    f(simulator
        .as_mut()
        .map(|x| <dyn Any>::downcast_mut(&mut **x).unwrap()))
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
