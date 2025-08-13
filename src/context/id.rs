use super::Context2;

/// A unique Id within a simulation.
#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Debug, Hash)]
pub struct Id(u64);

impl Id {
    pub fn new() -> Self {
        Context2::with(|cx| {
            let id = cx.pre_next_global_id.get() + 1;
            cx.pre_next_global_id.set(id);
            Id(id)
        })
    }
}
