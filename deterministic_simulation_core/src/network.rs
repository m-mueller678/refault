use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use crate::context::CONTEXT;
use crate::node::{current_node, get_node, Node, NodeId};
use crate::task::spawn;
use crate::time::sleep;

pub struct NetworkListenFuture {
    node: Node,
}

impl NetworkListenFuture {
    fn new() -> Self {
        let node = get_node(&current_node().unwrap());
        NetworkListenFuture { node }
    }
}

impl Future for NetworkListenFuture {
    type Output = NetworkPackage;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut node_waker = self.node.new_package_waker.lock().unwrap();
        if node_waker.is_some(){
            panic!("There already is another listener on this node.");
        }
        let mut packages = self.node.incoming_packages.lock().unwrap();
        if packages.is_empty() {
            *node_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(packages.remove(0))
    }
}

pub struct NetworkPackage {
    pub source: NodeId,
    pub destination: NodeId,
    pub message: String,
}

pub fn listen() -> NetworkListenFuture {
    NetworkListenFuture::new()
}

pub fn send(message: String, target: NodeId) {
    let current_node = current_node().unwrap();
    let package = NetworkPackage {
        message,
        source: current_node,
        destination: target,
    };
    let network = {
        let mut context = CONTEXT.lock().unwrap();
        context.as_mut().unwrap().network.clone()
    };
    network.transmit_package(package);
}

pub trait Network {
    fn transmit_package(&self, package: NetworkPackage);
}

pub(crate) struct DefaultNetwork {
}

impl Network for DefaultNetwork {
    fn transmit_package(&self, package: NetworkPackage){
        spawn(async move {
            sleep(Duration::from_millis(1000)).await;
            get_node(&current_node().unwrap()).receive_package(package);
        });
    }
}
