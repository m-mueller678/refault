use crate::context::with_context;
use crate::event::{Event, record_event};
use crate::executor::{ObservingFuture, Task, TaskTrackingFuture};
use crate::network::NetworkPackage;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

pub type NodeId = u64;

pub fn get_node(id: &NodeId) -> Node {
    with_context(|cx| {
        cx.nodes
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

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

impl Node {
    pub fn new() -> Node {
        with_context(|context| {
            let id = context.node_id_supplier.generate_id() as NodeId;
            let new_node = Node {
                id,
                incoming_messages: Arc::new(Mutex::new(vec![])),
                new_message_waker: Arc::new(Mutex::new(None)),
            };
            context.nodes.push(new_node.clone());
            new_node
        })
    }

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
    with_context(|context| context.current_node)
}

fn set_node(id: NodeId) {
    with_context(|context| {
        if context.current_node.is_some() {
            panic!("Node already set!");
        }
        context.current_node = Some(id);
    })
}

fn clear_node() {
    with_context(|context| {
        if context.current_node.is_some() {
            panic!("Node not set!");
        }
        context.current_node = None;
    })
}
