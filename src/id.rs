use std::{num::NonZeroU64, ops::RangeInclusive};

use super::Context2;

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
    pub fn new() -> Self {
        Context2::with(|cx| {
            let cx = cx.cx3();
            let id = cx.pre_next_global_id.get() + 1;
            cx.pre_next_global_id.set(id);
            Id(NonZeroU64::new(id).unwrap())
        })
    }
}

pub struct IdRange(RangeInclusive<NonZeroU64>);

impl IdRange {
    pub fn new(len: usize) -> Self {
        assert!(len > 0);
        Context2::with(|cx| {
            let cx = cx.cx3();
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

    pub fn get(&self, index: usize) -> Id {
        Id(self.0.start().checked_add(index as u64).unwrap())
    }
}
