use crate::{
    CONTEXT, Context, Context2, NodeId,
    event::EventHandler,
    executor::{Executor, ExecutorQueue},
    send_bind::ThreadAnchor,
};
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use std::{collections::HashMap, marker::PhantomData, time::Duration};

pub struct ContextInstallGuard(PhantomData<*const ()>);

impl ContextInstallGuard {
    pub fn new(event_handler: Box<dyn EventHandler>, seed: u64, start_time: Duration) -> Self {
        Context2::with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            assert!(
                cx2.thread_anchor
                    .replace(Some(ThreadAnchor::new()))
                    .is_none()
            );
            assert!(cx2.time.replace(Some(start_time)).is_none());
            assert!(
                cx2.rng
                    .replace(Some(ChaCha12Rng::seed_from_u64(seed)))
                    .is_none()
            );
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
            assert!(cx2.pre_next_global_id.get() == 0);
        });
        ContextInstallGuard(PhantomData)
    }

    pub fn destroy(&mut self) -> Option<Box<dyn EventHandler>> {
        CONTEXT.with(|cx2| {
            debug_assert!(cx2.current_node() == NodeId::INIT);
            cx2.time.get()?;
            assert!(cx2.queue.borrow().as_ref().unwrap().none_ready() || std::thread::panicking());
            Executor::final_stop();
            while let Some(x) = cx2.with_cx(|cx| cx.simulators.pop()) {
                drop(x.unwrap())
            }
            let Context { event_handler, .. } = cx2.context.take().unwrap();
            cx2.time.take().unwrap();
            cx2.rng.take().unwrap();
            Some(event_handler)
        })
    }
}

impl Drop for ContextInstallGuard {
    fn drop(&mut self) {
        self.destroy();
    }
}
