use crate::context::with_context;
use crate::executor::spawn;
use crate::time::sleep;
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

pub fn listen() -> NetworkListenFuture {
    with_context(|cx| {
        let node_id = cx.current_node;
        let node = cx.get_node(node_id);
        assert!(!node.has_listener)
    });
    node.has_listener = true;
    NetworkListenFuture(node_id)
}

impl Drop for NetworkListenFuture {
    fn drop(&mut self) {
        with_context(|cx| {
            let node = cx.get_node(self.0);
            node.has_listener = false;
            node.new_message_waker.clone_from(Waker::noop());
        })
    }
}
pub fn send(message: Box<dyn Any + Send + Sync + 'static>, target: NodeId) {
    let current_node = current_node().expect("Cannot send network message from outside a node!");
    let package = NetworkPackage {
        message,
        source: current_node,
        destination: target,
    };
    let network = with_context(|context| context.network.clone());
    network.transmit_message(package);
}

pub struct NetworkListenFuture(NodeId);

impl Future for NetworkListenFuture {
    type Output = NetworkPackage;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut messages = self.node.incoming_messages.lock().unwrap();
        if messages.is_empty() {
            *node_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(messages.remove(0))
    }
}

pub struct NetworkPackage {
    pub source: NodeId,
    pub destination: NodeId,
    pub message: Box<dyn Any + Send + Sync + 'static>,
}

pub trait Network {
    fn transmit_message(&self, package: NetworkPackage);
}

pub(crate) struct DefaultNetwork {}

impl Network for DefaultNetwork {
    fn transmit_message(&self, package: NetworkPackage) {
        spawn(async move {
            sleep(Duration::from_millis(1000)).await;
            get_node(&current_node().unwrap()).receive_message(package);
        });
    }
}
