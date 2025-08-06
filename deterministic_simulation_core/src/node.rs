use crate::context::{CONTEXT, Context};
use crate::event::record_event;
use crate::executor::{ObservingFuture, Task, TaskTrackingFuture};
use crate::network::NetworkPackage;
use std::cell::RefCell;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

pub type NodeId = u64;

thread_local! {
    pub(crate) static NODES: RefCell<Vec<Node>> = RefCell::new(vec!());
}

pub fn get_node(id: &NodeId) -> Node {
    NODES.with(|nodes| {
        nodes
            .borrow()
            .iter()
            .find(|x| x.id == *id)
            .unwrap_or_else(|| {
                panic!("Found no node with id {id}");
            })
            .clone()
    })
}

pub(crate) struct NodeIdSupplier {
    next_id: u64,
}

impl NodeIdSupplier {
    pub(crate) fn new() -> NodeIdSupplier {
        Self { next_id: 0 }
    }

    pub(crate) fn generate_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[derive(Clone)]
pub struct Node {
    pub id: NodeId,
    pub(crate) incoming_messages: Arc<Mutex<Vec<NetworkPackage>>>,
    pub(crate) new_message_waker: Arc<Mutex<Option<Waker>>>,
}

impl Node {
    pub fn new() -> Node {
        let mut context = CONTEXT.lock().unwrap();
        let id = context.as_mut().unwrap().node_id_supplier.generate_id() as NodeId;
        let new_node = Node {
            id,
            incoming_messages: Arc::new(Mutex::new(vec![])),
            new_message_waker: Arc::new(Mutex::new(None)),
        };

        NODES.with(|nodes| {
            nodes.borrow_mut().push(new_node.clone());
        });

        new_node
    }

    pub fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + 'static + Send + Sync,
    ) -> TaskTrackingFuture<T> {
        record_event(Box::new(NodeSpawnedEvent { node_id: self.id }));

        let state = Arc::new(Mutex::new(crate::executor::TaskTrackingFutureState::new()));
        let observing_future = ObservingFuture {
            state: state.clone(),
            inner: Mutex::new(Box::pin(future)),
        };

        let task = Arc::new(Task::new(NodeAwareFuture {
            id: self.id,
            inner: Mutex::new(Box::pin(observing_future)),
        }));
        let mut mutex_guard = CONTEXT.lock().unwrap();
        let context: &mut Context = mutex_guard.as_mut().unwrap();
        context.executor.queue(task);

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

struct NodeSpawnedEvent {
    node_id: NodeId,
}

impl Display for NodeSpawnedEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeSpawnedEvent{{node_id: {}}}", self.node_id)
    }
}

pub(crate) struct NodeAwareFuture {
    pub(crate) id: NodeId,
    pub(crate) inner: Mutex<Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>>,
}

impl Future for NodeAwareFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        set_node(self.id);
        let result = self.inner.lock().unwrap().as_mut().poll(cx);
        clear_node();
        result
    }
}

pub fn current_node() -> Option<NodeId> {
    let guard = CONTEXT.lock().unwrap();
    guard.as_ref()?.current_node
}

fn set_node(id: NodeId) {
    let mut guard = CONTEXT.lock().unwrap();
    let context = guard.as_mut().unwrap();
    if context.current_node == Some(id) {
        panic!("Node already set!");
    }
    context.current_node = Some(id);
}

fn clear_node() {
    let mut guard = CONTEXT.lock().unwrap();
    let context = guard.as_mut().unwrap();
    if context.current_node == None {
        panic!("Node not set!");
    }
    context.current_node = None;
}

pub(crate) fn reset_nodes() {
    NODES.with(|nodes| {
        nodes.borrow_mut().clear();
    });
}
