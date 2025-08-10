use std::{
    any::{Any, TypeId},
    cell::Cell,
    collections::{HashMap, VecDeque},
    pin::Pin,
    task::{Poll, Waker},
    time::Instant,
};

use either::Either::{Left, Right};

use crate::{
    context::NodeId,
    runtime::spawn,
    simulator::{Simulator, with_simulator},
    time::sleep_until,
};

pub trait Packet: Any {}

pub trait ConnectivityFunction {
    fn send_connectivity(&mut self, packet: &WrappedPacket) -> SendConnectivity;
    fn pre_receive_connectivity(&mut self, dst: NodeId) -> PreReceiveConnectivity;
    fn receive_connectivity(&mut self, packet: &WrappedPacket) -> ReceiveConnectivity;
}

pub struct WrappedPacket {
    src: NodeId,
    dst: NodeId,
    id: PacketId,
    content: Cell<Option<Box<dyn Packet>>>,
}

impl WrappedPacket {
    pub fn put_packet(&self, x: Box<dyn Packet>) {
        assert!(self.content.replace(Some(x)).is_none());
    }
    pub fn take_packet(&self) -> Box<dyn Packet> {
        self.content.take().unwrap()
    }
    pub fn src(&self) -> NodeId {
        self.src
    }
    pub fn dst(&self) -> NodeId {
        self.dst
    }
    pub fn id(&self) -> PacketId {
        self.id
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
pub struct PacketId(u64);

pub enum SendConnectivity {
    RetryLater(Pin<Box<dyn Future<Output = ()>>>),
    Drop,
    Error { error: std::io::Error },
    Deliver { deliver_at: Instant },
}

pub enum PreReceiveConnectivity {
    /// Receive a packet if any, wait otherwise.
    Continue,
    /// Return an error.
    /// The queue of incoming packages is not modified.
    Error { error: std::io::Error },
}

pub enum ReceiveConnectivity {
    Receive,
    ErrorDiscard { error: std::io::Error },
    ErrorRestore { error: std::io::Error },
    SilentDiscard,
}

pub struct PacketNetwork {
    connectivity: Box<dyn ConnectivityFunction>,
    messages: HashMap<(NodeId, TypeId), (Option<Waker>, VecDeque<WrappedPacket>)>,
    pre_next_id: u64,
}

impl PacketNetwork {
    pub fn new(connectivity: impl ConnectivityFunction + 'static) -> Self {
        PacketNetwork {
            connectivity: Box::new(connectivity),
            messages: HashMap::default(),
            pre_next_id: 0,
        }
    }
    fn enqueue_message(&mut self, mut packet: WrappedPacket) {
        let key = (
            packet.dst,
            packet_type_id(&**packet.content.get_mut().as_mut().unwrap()),
        );
        let messages = self.messages.entry(key).or_default();
        messages.1.push_back(packet);
        if let Some(w) = messages.0.as_ref() {
            w.wake_by_ref();
        }
    }
}

impl Simulator for PacketNetwork {}

pub fn packet_type_id(p: &dyn Packet) -> TypeId {
    (*p).type_id()
}

pub async fn send_packet(dst: NodeId, content: Box<dyn Packet>) -> Result<(), std::io::Error> {
    let mut packet = WrappedPacket {
        src: NodeId::current(),
        dst,
        id: PacketId(0),
        content: Cell::new(Some(content)),
    };
    loop {
        let result = with_simulator::<PacketNetwork, _>(|net| {
            if packet.id.0 == 0 {
                net.pre_next_id += 1;
                packet.id = PacketId(net.pre_next_id);
            }
            match net.connectivity.send_connectivity(&packet) {
                SendConnectivity::RetryLater(future) => Right((future, packet)),
                SendConnectivity::Drop => Left(Ok(())),
                SendConnectivity::Error { error } => Left(Err(error)),
                SendConnectivity::Deliver { deliver_at } => {
                    spawn(async move {
                        sleep_until(deliver_at).await;
                        with_simulator::<PacketNetwork, _>(|net| net.enqueue_message(packet));
                    });
                    Left(Ok(()))
                }
            }
        });
        match result {
            Left(x) => return x,
            Right((fut, p)) => {
                packet = p;
                fut.await;
            }
        }
    }
}

struct ReceiveFuture {
    type_id: TypeId,
    node_id: NodeId,
    waker_registered: bool,
}

impl Future for ReceiveFuture {
    type Output = Result<(NodeId, Box<dyn Packet>), std::io::Error>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        with_simulator::<PacketNetwork, _>(|net| {
            match net.connectivity.pre_receive_connectivity(self.node_id) {
                PreReceiveConnectivity::Continue => (),
                PreReceiveConnectivity::Error { error } => return Poll::Ready(Err(error)),
            }
            let messages = net
                .messages
                .entry((self.node_id, self.type_id))
                .or_default();
            loop {
                if let Some(msg) = messages.1.pop_front() {
                    let result = match net.connectivity.receive_connectivity(&msg) {
                        ReceiveConnectivity::Receive => {
                            Ok((msg.src, msg.content.into_inner().unwrap()))
                        }
                        ReceiveConnectivity::ErrorDiscard { error } => Err(error),
                        ReceiveConnectivity::ErrorRestore { error } => {
                            messages.1.push_front(msg);
                            Err(error)
                        }
                        ReceiveConnectivity::SilentDiscard => {
                            continue;
                        }
                    };
                    if self.waker_registered {
                        messages.0 = None;
                        self.waker_registered = false;
                    }
                    break Poll::Ready(result);
                } else {
                    if let Some(w) = &mut messages.0 {
                        assert!(
                            self.waker_registered,
                            "multiple receive futures waiting on same node and type"
                        );
                        w.clone_from(cx.waker())
                    } else {
                        messages.0 = Some(cx.waker().clone());
                        self.waker_registered = true;
                    }
                    break Poll::Pending;
                }
            }
        })
    }
}

impl Drop for ReceiveFuture {
    fn drop(&mut self) {
        if self.waker_registered {
            with_simulator::<PacketNetwork, _>(|net| {
                let messages = net.messages.get_mut(&(self.node_id, self.type_id)).unwrap();
                let removed = messages.0.take().is_some();
                debug_assert!(removed);
            })
        }
    }
}

pub async fn receive<T: Packet>() -> Result<(NodeId, Box<T>), std::io::Error> {
    let received = ReceiveFuture {
        type_id: TypeId::of::<T>(),
        node_id: NodeId::current(),
        waker_registered: false,
    }
    .await?;
    Ok((received.0, (received.1 as Box<dyn Any>).downcast().unwrap()))
}
