//! Simulation-scoped singletons.
//!
//! Within a simulation global objects implementing the [Simulator] trait may be registered and accessed.
//! These can be used to simulate some external system the program needs to interact with.
//! For instance, the [PacketNetwork](crate::packet_network::PacketNetwork) simulator contains global state to implement a virtual network between nodes.
//!
//! A simulator can be registered using [add_simulator] and later accessed using [with_simulator].
//! At the end of a simulation, all associated simulators will be destroyed.
//! There is at most one object for each simulator type in a simulation.

use crate::{
    check_send::{CheckSend, Constraint, SimBound},
    context::{Context2, executor::NodeId, with_context},
};
use scopeguard::guard;
use std::{
    any::{Any, TypeId, type_name},
    cell::RefCell,
    marker::PhantomData,
    mem,
    rc::Rc,
};

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
/// Simulators must be added before additional nodes are spawned.
///
/// Panics if any of the following is true:
/// - A simulator with the same type has already been added
/// - An additional node has already been created
/// - The simulation is shutting down.
pub fn add_simulator<S: Simulator>(simulator: S) {
    let simulator = Some(Box::new(simulator) as Box<_>);
    with_context(|cx| {
        assert!(cx.executor.node_count() == 1 && !cx.executor.is_final_sopping());
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
    if let Some((index, simulator)) = with_context(|cx| {
        let index = *cx.simulators_by_type.get(&TypeId::of::<S>())?;

        let simulator = cx.simulators[index]
            .take()
            .unwrap_or_else(|| panic!("simulator already mutably borrowed: {}", type_name::<S>()));
        Some((index, simulator))
    }) {
        let mut simulator = guard(simulator, |simulator| {
            with_context(|cx| {
                assert!(cx.simulators[index].replace(simulator).is_none());
            });
        });
        let simulator: &mut dyn Simulator = &mut **simulator;
        f(Some((simulator as &mut dyn Any).downcast_mut().unwrap()))
    } else {
        f(None)
    }
}

pub struct SimulatorHandle<S: Simulator>(CheckSend<PhantomData<Rc<RefCell<S>>>, SimBound>);

impl<S: Simulator> Clone for SimulatorHandle<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Get a handle for the simulator
///
/// Panics if no simulator of this type has been added.
pub fn simulator<S: Simulator>() -> SimulatorHandle<S> {
    with_context(|cx| {
        if !cx.simulators_by_type.contains_key(&TypeId::of::<S>()) {
            panic!("simulator does not exist: {}", type_name::<S>());
        }
    });
    SimulatorHandle(SimBound::wrap(PhantomData))
}

impl<S: Simulator> SimulatorHandle<S> {
    /// Call `f` with a mutable reference to the simulator.
    ///
    /// This acquires an exclusive lock on the simulator.
    /// Attempting to acquire another reference to the same simulator within `f` will panic.
    pub fn with<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        let _: PhantomData<_> = *self.0;
        with_simulator_option(|mut x| f(x.as_mut().unwrap()))
    }
}

impl<S: NodeSimulator> SimulatorHandle<PerNode<S>> {
    /// Call `f` with a mutable reference to the node simulator of the specified node.
    ///
    /// See [Self::with] for concerns about locking.
    pub fn with_node<R>(&self, node: NodeId, f: impl FnOnce(&mut S) -> R) -> R {
        self.with(|s| f(&mut s.simulators[node.0]))
    }

    /// Call `f` with a mutable reference to the node simulator of the current node.
    ///
    /// See [Self::with] for concerns about locking.
    pub fn with_current_node<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        self.with_node(NodeId::current(), f)
    }
}

pub(crate) fn for_all_simulators(
    cx2: &Context2,
    forward: bool,
    mut f: impl FnMut(&mut dyn Simulator),
) {
    let len = cx2.with_cx(|cx| cx.simulators.len());
    let mut simulator = None;
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
}

/// A node-scoped singleton.
///
/// This is a node-scoped version of [Simulator].
/// Use this together with [PerNode].
pub trait NodeSimulator: 'static {
    fn stop_node(&mut self);
    fn start_node(&mut self);
}

/// Node-scoped singletons.
///
/// This simulator contains one `NodeSimulator` for each node in the simulation.
/// The node simulators can be accessed using [SimulatorHandle::with_node] and [SimulatorHandle::with_current_node].
pub struct PerNode<S> {
    simulators: Vec<S>,
    new: Box<dyn FnMut() -> S>,
}

impl<S> std::ops::Index<NodeId> for PerNode<S> {
    type Output = S;

    fn index(&self, index: NodeId) -> &Self::Output {
        &self.simulators[index.0]
    }
}

impl<S> std::ops::IndexMut<NodeId> for PerNode<S> {
    fn index_mut(&mut self, index: NodeId) -> &mut Self::Output {
        &mut self.simulators[index.0]
    }
}

impl<S: NodeSimulator> Simulator for PerNode<S> {
    fn create_node(&mut self) {
        self.simulators.push((self.new)())
    }

    fn stop_node(&mut self) {
        self.simulators[NodeId::current().0].stop_node();
    }

    fn start_node(&mut self) {
        self.simulators[NodeId::current().0].start_node();
    }
}

impl<S: NodeSimulator> PerNode<S> {
    pub fn new(mut new: Box<dyn FnMut() -> S>) -> Self {
        Self {
            simulators: vec![new()],
            new,
        }
    }
}
