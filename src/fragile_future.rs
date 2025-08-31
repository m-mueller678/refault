use crate::runtime::NodeId;
use fragile::Fragile;
use std::{
    ops::{Deref, DerefMut},
    pin::Pin,
};

pub trait Constraint: Private {
    fn wrap<T>(x: T) -> Fragile2<T, Self> {
        Fragile2::new(x)
    }
}

trait Private: Sized {
    fn check(&self);
    fn new() -> Self;
}

pub struct NodeBound(NodeId);
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

pub struct Fragile2<T, C: Constraint>(Fragile<(T, C)>);

impl<T, C: Constraint> Deref for Fragile2<T, C> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let x = self.0.get();
        x.1.check();
        &x.0
    }
}

impl<T, C: Constraint> DerefMut for Fragile2<T, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let x = self.0.get_mut();
        x.1.check();
        &mut x.0
    }
}

impl<F: Future, C: Constraint> Future for Fragile2<F, C> {
    type Output = F::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        Self::as_pin_mut(self).poll(cx)
    }
}

impl<T, C: Constraint> Fragile2<T, C> {
    fn new(x: T) -> Self {
        Fragile2(Fragile::new((x, C::new())))
    }
    pub fn as_pin_mut(this: Pin<&mut Self>) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(&mut *Pin::get_unchecked_mut(this)) }
    }

    pub fn as_pin_ref(this: Pin<&Self>) -> Pin<&T> {
        unsafe { Pin::new_unchecked(&*this.get_ref()) }
    }
}
