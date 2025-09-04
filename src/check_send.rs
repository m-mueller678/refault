use crate::runtime::NodeId;
use fragile::Fragile;
use std::{
    ops::{Deref, DerefMut},
    pin::Pin,
};

#[allow(private_bounds)]
pub trait Constraint: Private {
    fn wrap<T>(x: T) -> CheckSend<T, Self> {
        CheckSend::new(x)
    }
}

trait Private: Clone + Sized {
    fn check(&self);
    fn new() -> Self;
}

#[derive(Clone)]
pub struct NodeBound(NodeId);
#[derive(Clone)]
pub struct SimBound(());

impl Private for NodeBound {
    fn check(&self) {
        assert_eq!(NodeId::current(), self.0)
    }
    fn new() -> Self {
        NodeBound(NodeId::current())
    }
}
impl Private for SimBound {
    fn check(&self) {}
    fn new() -> Self {
        SimBound(())
    }
}
impl Constraint for NodeBound {}
impl Constraint for SimBound {}

#[derive(Clone)]
pub struct CheckSend<T, C: Constraint>(Fragile<(T, C)>);

impl<T> CheckSend<T, NodeBound> {
    pub fn unwrap_check_send_node(self) -> T {
        self.unwrap_check_send()
    }
}

impl<T> CheckSend<T, SimBound> {
    pub fn unwrap_check_send_sim(self) -> T {
        self.unwrap_check_send()
    }
}

impl<T, C: Constraint> Deref for CheckSend<T, C> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let x = self.0.get();
        x.1.check();
        &x.0
    }
}

impl<T, C: Constraint> DerefMut for CheckSend<T, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let x = self.0.get_mut();
        x.1.check();
        &mut x.0
    }
}

impl<F: Future, C: Constraint> Future for CheckSend<F, C> {
    type Output = F::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        Self::as_pin_mut(self).poll(cx)
    }
}

impl<T, C: Constraint> CheckSend<T, C> {
    fn new(x: T) -> Self {
        CheckSend(Fragile::new((x, C::new())))
    }
    pub fn as_pin_mut(this: Pin<&mut Self>) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(&mut *Pin::get_unchecked_mut(this)) }
    }

    pub fn as_pin_ref(this: Pin<&Self>) -> Pin<&T> {
        unsafe { Pin::new_unchecked(&**this.get_ref()) }
    }
    fn unwrap_check_send(self) -> T {
        let this = self.0.into_inner();
        this.1.check();
        this.0
    }
}
