//! Unique Id generation.

use std::{num::NonZeroU64, ops::RangeInclusive};

use super::SimCx;

/// A unique Id within a simulation.
///
/// The `Default` implementation returns a new id.
#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Id(NonZeroU64);

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl Id {
    /// Generate a new unique id.
    pub fn new() -> Self {
        SimCx::with(|cx| {
            let cx = cx.cxu();
            let id = cx.pre_next_global_id.get() + 1;
            cx.pre_next_global_id.set(id);
            Id(NonZeroU64::new(id).unwrap())
        })
    }

    #[cfg(feature = "emit-tracing")]
    /// Turn this id into a value that can be recorded with [tracing].
    pub fn tv(&self) -> impl tracing::Value {
        self.0.get()
    }
}

/// A range of [Ids](Id).
///
/// This is equivalent to a `Vec<Id>`, but more efficient.
pub struct IdRange(RangeInclusive<NonZeroU64>);

impl IdRange {
    /// Generate a new range of [Ids](Id).
    ///
    /// Panics if `len == 0`.
    pub fn new(len: usize) -> Self {
        assert!(len > 0);
        SimCx::with(|cx| {
            let cx = cx.cxu();
            let last = cx.pre_next_global_id.get() + len as u64;
            cx.pre_next_global_id.set(last);
            let first = last - (len as u64 - 1);
            IdRange(NonZeroU64::new(first).unwrap()..=NonZeroU64::new(last).unwrap())
        })
    }

    // range is always non-empty
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        (self.0.end().get() - self.0.start().get() + 1) as usize
    }

    /// Get the [Id] at index `index`.
    pub fn get(&self, index: usize) -> Id {
        Id(self.0.start().checked_add(index as u64).unwrap())
    }
}
