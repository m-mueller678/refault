use std::fmt::{Display, Formatter};
use crate::context::{Context, CONTEXT};
use crate::network::NetworkPackage;
use crate::task::{ObservingFuture, Task, TaskTrackingFuture};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use crate::event::record_event;

pub type NodeId = u64;

pub(crate) static NODES: Mutex<Vec<Node>> = Mutex::new(vec!());

pub fn get_node(id: &NodeId) -> Node {
    {
        let mutex_guard = NODES.lock().unwrap();
        for node in mutex_guard.iter() {
            return node.clone();
        }
    }
    panic!("Found no node with id {id}");
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
    pub(crate) incoming_packages: Arc<Mutex<Vec<NetworkPackage>>>,
    pub(crate) new_package_waker: Arc<Mutex<Option<Waker>>>,
}

impl Node {
    pub fn new() -> Node {
        let mut context_binding = CONTEXT.lock().unwrap();
        let id = context_binding.as_mut().unwrap().node_id_supplier.generate_id() as NodeId;
        let new_node = Node {
            id,
            incoming_packages: Arc::new(Mutex::new(vec!())),
            new_package_waker: Arc::new(Mutex::new(None)),
        };
        NODES.lock().unwrap().push(new_node.clone());
        new_node
    }

    pub fn spawn<T: Send + 'static>(&self, future: impl Future<Output=T> + 'static + Send + Sync) -> TaskTrackingFuture<T> {
        record_event(Box::new(NodeSpawnedEvent{node_id: self.id}));
        
        let state = Arc::new(Mutex::new(crate::task::TaskTrackingFutureState::new()));
        let observing_future = ObservingFuture {
            state: state.clone(),
            inner: Mutex::new(Box::pin(future)),
        };

        let task = Arc::new(Task::new(NodeAwareFuture {
            id: self.id,
            inner: Mutex::new(Box::pin(observing_future)),
        }));
        let mut binding = CONTEXT.lock().unwrap();
        let context: &mut Context = binding.as_mut().unwrap();
        context.executor.queue(task);

        TaskTrackingFuture {
            inner: state,
        }
    }

    pub fn receive_package(&self, package: NetworkPackage) {
        self.incoming_packages.lock().unwrap().push(package);
        let mut waker_option = self.new_package_waker.lock().unwrap();
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
    pub(crate) inner: Mutex<Pin<Box<dyn Future<Output=()> + Send + Sync + 'static>>>,
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
