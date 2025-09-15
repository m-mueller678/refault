use crate::{
    CONTEXT, Context, Context2, Context3,
    event::EventHandler,
    executor::{Executor, ExecutorQueue},
    node_id::NodeId,
    send_bind::ThreadAnchor,
};
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    marker::PhantomData,
    time::Duration,
};

pub struct ContextInstallGuard(PhantomData<*const ()>);

impl ContextInstallGuard {
    pub fn new(event_handler: Box<dyn EventHandler>, seed: u64, start_time: Duration) -> Self {
        Context2::with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            cx2.cx3
                .set(Context3 {
                    current_node: Cell::new(NodeId::INIT),
                    thread_anchor: ThreadAnchor::new(),
                    pre_next_global_id: Cell::new(0),
                    time: Cell::new(start_time),
                    rng: RefCell::new(ChaCha12Rng::seed_from_u64(seed)),
                })
                .ok()
                .unwrap();
            assert!(
                cx2.queue
                    .borrow_mut()
                    .replace(ExecutorQueue::new())
                    .is_none()
            );
            let new_context = Context {
                executor: cx2.queue.borrow_mut().as_mut().unwrap().executor(),
                event_handler,
                // random is already deterministic at this point.
                simulators: Vec::new(),
                simulators_by_type: HashMap::new(),
            };
            assert!(cx2.context.borrow_mut().replace(new_context).is_none());
        });
        ContextInstallGuard(PhantomData)
    }

    pub fn destroy(&mut self) -> Option<Box<dyn EventHandler>> {
        CONTEXT.with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            cx2.context.borrow_mut().as_ref()?;
            assert!(cx2.queue.borrow().as_ref().unwrap().none_ready() || std::thread::panicking());
            Executor::final_stop();
            while let Some(x) = cx2.with_cx(|cx| cx.simulators.pop()) {
                drop(x.unwrap())
            }
            let Context { event_handler, .. } = cx2.context.take().unwrap();
            Some(event_handler)
        })
    }
}

impl Drop for ContextInstallGuard {
    fn drop(&mut self) {
        self.destroy();
    }
}
