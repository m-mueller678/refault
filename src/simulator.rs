use std::{
    any::{Any, TypeId},
    cell::RefCell,
    rc::Rc,
};

use crate::context::{NodeId, with_context};

pub trait Simulator: Any {
    fn node_created(&mut self, id: NodeId);
}

pub fn add_simulator<S: Simulator>(s: S) {
    with_context(|cx| {
        assert!(
            cx.simulators
                .insert(TypeId::of::<S>(), Rc::new(RefCell::new(s)))
                .is_none()
        );
    })
}

pub fn with_simulator<S: Simulator, R>(f: impl FnOnce(Option<&mut S>) -> R) -> R {
    let simulator = with_context(|cx| cx.simulators.get(&TypeId::of::<S>()).cloned());
    let mut simulator = simulator.as_ref().map(|x| x.borrow_mut());
    f(simulator
        .as_mut()
        .map(|x| <dyn Any>::downcast_mut(&mut **x).unwrap()))
}
