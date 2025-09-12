use crate::send_bind::{NodeBound, SimBound};
use std::pin::Pin;

pub type SimNodeBound<T> = SimBound<NodeBound<T>>;

impl<T> From<T> for SimBound<NodeBound<T>> {
    fn from(value: T) -> Self {
        SimBound::from(NodeBound::from(value))
    }
}

pub trait SimNodeBoundExt: Sized {
    fn sim_node_bound(self) -> SimNodeBound<Self> {
        self.into()
    }
    fn node_bound(self) -> NodeBound<Self> {
        self.into()
    }
    fn sim_bound(self) -> SimBound<Self> {
        self.into()
    }
}

impl<T> SimNodeBoundExt for T {}

macro_rules! impl_traits {
    ($W:ident) => {
        impl<T: Future> Future for $W<T> {
            type Output = T::Output;

            fn poll(
                self: Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                Self::as_pin_mut(self).poll(cx)
            }
        }
    };
}

impl_traits!(SimBound);
impl_traits!(NodeBound);
