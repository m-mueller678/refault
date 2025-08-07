use crate::event::{Event, record_event};
use crate::executor::{ObservingFuture, Task, TaskTrackingFuture};
use crate::network::NetworkPackage;
use futures::task::AtomicWaker;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use crate::event::EventHandler;
use crate::executor::Executor;
use crate::network::Network;
use rand_chacha::ChaCha12Rng;
use std::cell::RefCell;
thread_local! {

static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn with_context_option<R>(f: impl FnOnce(&mut Option<Context>) -> R) -> R {
    CONTEXT.with(|c| f(&mut c.borrow_mut()))
}
pub fn with_context<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    with_context_option(|cx| f(cx.as_mut().expect("no context set")))
}
pub fn run_with_context<R>(context: Context, f: impl FnOnce() -> R) -> (R, Context) {
    with_context_option(|c| assert!(c.replace(context).is_none()));
    let r = f();
    let context = with_context_option(|c| c.take().unwrap());
    (r, context)
}
pub struct Context {
    pub executor: Arc<Executor>,
    pub current_node: Option<NodeId>,
    pub event_handler: Box<dyn EventHandler>,
    pub random_generator: ChaCha12Rng,
    pub simulation_start_time: u64,
    pub network: Arc<dyn Network + Send + Sync>,
    pub nodes: Vec<Node>,
}
#[derive(Eq, Debug, PartialEq, Clone, Copy)]
pub struct NodeId(usize);
pub struct Node {
    pub id: NodeId,
    pub has_listener: bool,
    pub incoming_messages: Vec<NetworkPackage>,
    pub new_message_waker: AtomicWaker,
}

impl Context {
    pub fn get_node(&mut self, i: NodeId) -> &mut Node {
        &mut self.nodes[i.0]
    }
    pub fn new_node(&mut self) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            id,
            has_listener: false,
            incoming_messages: Default::default(),
            new_message_waker: AtomicWaker::new(),
        });
        id
    }
    pub fn current_node(&mut self) -> Option<NodeId> {
        self.current_node
    }
    fn set_current_node(&mut self, id: Option<NodeId>) {
        assert!(self.current_node.is_none() ^ id.is_none());
        self.current_node = id;
    }
}
impl Node {
    pub fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + 'static + Send + Sync,
    ) -> TaskTrackingFuture<T> {
        record_event(Event::NodeSpawnedEvent { node_id: self.id });

        let state = Arc::new(Mutex::new(crate::executor::TaskTrackingFutureState::new()));
        let observing_future = ObservingFuture {
            state: state.clone(),
            inner: Mutex::new(Box::pin(future)),
        };

        let task = Arc::new(Task::new(NodeAwareFuture {
            id: self.id,
            inner: Mutex::new(Box::pin(observing_future)),
        }));
        with_context(|context| context.executor.queue(task));
        TaskTrackingFuture { inner: state }
    }

    pub fn receive_message(&self, message: NetworkPackage) {
        self.incoming_messages.lock().unwrap().push(message);
        let mut waker_option = self.new_message_waker.lock().unwrap();
        if let Some(waker) = waker_option.take() {
            waker.wake();
        }
    }
}

pub(crate) struct NodeAwareFuture<F> {
    pub id: NodeId,
    pub inner: F,
}

impl<F: Future> Future for NodeAwareFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        with_context(|cx| cx.set_node(Some(self.id)));
        let result = self.inner.poll(cx);
        with_context(|cx| cx.set_node(None));
        result
    }
}
