use std::ops::RangeInclusive;

use super::Context2;

/// A unique Id within a simulation.
///
/// The `Default` implementation returns a new id.
#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Debug, Hash)]
pub struct Id(u64);

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl Id {
    pub fn new() -> Self {
        Context2::with(|cx| {
            let id = cx.pre_next_global_id.get() + 1;
            cx.pre_next_global_id.set(id);
            Id(id)
        })
    }
}

pub struct IdRange(RangeInclusive<u64>);

impl IdRange {
    pub fn new(len: usize) -> Self {
        assert!(len > 0);
        Context2::with(|cx| {
            let last = cx.pre_next_global_id.get() + len as u64;
            cx.pre_next_global_id.set(last);
            IdRange(last - (len as u64 - 1)..=last)
        })
    }

    // range is always non-empty
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        (self.0.end() - self.0.start() + 1) as usize
    }

    pub fn get(&self, index: usize) -> Id {
        Id(self.0.start() + index as u64)
    }
}
