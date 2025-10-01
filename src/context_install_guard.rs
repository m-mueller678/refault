use crate::{
    SimCx, SimCxl, SimCxu,
    event::EventHandler,
    executor::{ExecutorQueue, stop_simulation},
    node_id::NodeId,
    sim_builder::SimulationOutput,
};
use futures::never::Never;
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
        SimCx::with(|cx| {
            cx.once_cell
                .set(SimCxu {
                    current_node: Cell::new(NodeId::INIT),
                    #[cfg(feature = "send-bind")]
                    thread_anchor: crate::send_bind::ThreadAnchor::new(),
                    pre_next_global_id: Cell::new(0),
                    time: Cell::new(start_time),
                    rng: RefCell::new(ChaCha12Rng::seed_from_u64(seed)),
                })
                .ok()
                .unwrap();
            let old_queue = cx.queue.borrow_mut().replace(ExecutorQueue::new());
            assert!(old_queue.is_none());
            let old_context = cx.context.borrow_mut().replace(SimCxl {
                executor: cx.queue.borrow_mut().as_mut().unwrap().executor(),
                event_handler,
                simulators: Vec::new(),
                simulators_by_type: HashMap::new(),
            });
            assert!(old_context.is_none());
        });
        ContextInstallGuard(PhantomData)
    }

    pub fn destroy(&mut self, finalize_events: bool) -> Option<SimulationOutput<Never>> {
        SimCx::with(|cx| {
            debug_assert!(cx.current_node() == NodeId::INIT);
            cx.context.borrow_mut().as_ref()?;
            let time = cx.cxu().time.get();
            assert!(
                cx.queue.borrow().as_ref().unwrap().none_ready()
                    || std::thread::panicking()
                    || cx.with_cx(|cx| cx.executor.is_final_stopping())
            );
            stop_simulation();
            while let Some(x) = cx.with_cx(|cx| cx.simulators.pop()) {
                drop(x.unwrap())
            }
            let SimCxl { event_handler, .. } = cx.context.take().unwrap();
            Some(SimulationOutput {
                // this is later adjusted by simbuilder
                time_elapsed: time,
                output: None,
                events: if finalize_events {
                    event_handler.finalize()
                } else {
                    Box::new(())
                },
            })
        })
    }
}

impl Drop for ContextInstallGuard {
    fn drop(&mut self) {
        self.destroy(false);
    }
}
