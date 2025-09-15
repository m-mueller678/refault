pub use crate::send_bind_util::*;
use crate::{Context2, runtime::NodeId};
use scopeguard::guard;
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    num::NonZeroU64,
    ops::{Deref, DerefMut},
    panic::abort_unwind,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

#[derive(Clone)]
pub struct NodeBound<T> {
    node: NodeId,
    inner: ManuallyDrop<T>,
}

impl<T> Drop for NodeBound<T> {
    fn drop(&mut self) {
        let guard = guard::<&mut Self, _>(self, |x| unsafe {
            // We must drop the value even if it moved to a different node to uphold the safety invariants of Pin.
            std::ptr::drop_in_place::<T>(&mut *x.inner);
        });
        guard.check();
    }
}

impl<T> NodeBound<T> {
    fn check(&self) {
        assert_eq!(self.node, NodeId::current());
    }

    pub fn unwrap_node_bound(mut self) -> T {
        self.check();
        let ret = unsafe {
            // # Safety
            // `self.inner` is not used again as we immediately forget it after.
            ManuallyDrop::take(&mut self.inner)
        };
        mem::forget(self);
        ret
    }
}

#[derive(Clone)]
pub struct SimBound<T> {
    inner: ManuallyDrop<T>,
    id: NonZeroU64,
}

// # Safety
// All uses of the possible unsend/unsync inner value happen after checking we are still on the same thread, including dropping it.
unsafe impl<T> Send for SimBound<T> {}
unsafe impl<T> Sync for SimBound<T> {}

impl<T> Drop for SimBound<T> {
    fn drop(&mut self) {
        // We must not unwind without dropping inner to uphold the safety invariants of Pin.
        abort_unwind(|| {
            self.check();
        });
        unsafe {
            std::ptr::drop_in_place::<T>(&mut *self.inner);
        }
    }
}
impl<T> SimBound<T> {
    fn current_anchor() -> NonZeroU64 {
        Context2::with(|cx| cx.thread_anchor().unwrap().id)
    }

    pub fn unwrap_sim_bound(mut self) -> T {
        self.check();
        let ret = unsafe {
            // # Safety
            // `self.inner` is not used again as we immediately forget it after.
            ManuallyDrop::take(&mut self.inner)
        };
        mem::forget(self);
        ret
    }

    fn check(&self) {
        assert_eq!(self.id, Self::current_anchor());
    }
}

impl<T> From<T> for NodeBound<T> {
    fn from(value: T) -> Self {
        NodeBound {
            node: NodeId::current(),
            inner: ManuallyDrop::new(value),
        }
    }
}
impl<T> From<T> for SimBound<T> {
    fn from(value: T) -> Self {
        SimBound {
            id: Self::current_anchor(),
            inner: ManuallyDrop::new(value),
        }
    }
}

/// Unique id that cannot be moved between threads.
/// There can never be two anchros with the same id on different threads.
#[derive(Clone, Copy)]
pub(crate) struct ThreadAnchor {
    id: NonZeroU64,
    _unsend: PhantomData<*const u8>,
}

impl ThreadAnchor {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT
            .fetch_update(Relaxed, Relaxed, |x| x.checked_add(1))
            .unwrap()
            + 1;
        ThreadAnchor {
            id: NonZeroU64::new(id).unwrap(),
            _unsend: PhantomData,
        }
    }
}

macro_rules! impl_traits {
    ($W:ident) => {
        impl<T> $W<T> {
            // # Safety
            // - It is impossible to move out of inner through a pined refernce as we never provide a mutable reference to it,
            // except from an unpinned mutable reference to the wrapper type.
            // - When drop is called on the wrapper, it always does one of the following:
            //   - drop the contained value in place
            //   - abort the process
            // In either case, the safety invariants of Pin are upheld.
            pub fn as_pin_mut(this: Pin<&mut Self>) -> Pin<&mut T> {
                unsafe { Pin::new_unchecked(&mut *this.get_unchecked_mut()) }
            }

            pub fn as_pin_ref(this: Pin<&Self>) -> Pin<&T> {
                unsafe { Pin::new_unchecked(&*this.get_ref()) }
            }
        }

        impl<T> Deref for $W<T> {
            type Target = T;

            fn deref(&self) -> &Self::Target {
                self.check();
                &self.inner
            }
        }

        impl<T> DerefMut for $W<T> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.check();
                &mut self.inner
            }
        }
    };
}

impl_traits!(SimBound);
impl_traits!(NodeBound);
