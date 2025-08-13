use crate::{
    context::NodeId,
    runtime::spawn,
    simulator::{Simulator, SimulatorHandle, simulator},
    time::sleep_until,
};
use futures::{Sink, SinkExt, Stream};
use futures_channel::mpsc;
use std::{
    any::{Any, TypeId},
    collections::{HashMap, hash_map::Entry},
    io::{Error, ErrorKind},
    pin::Pin,
    task::Poll,
    time::Instant,
};
use std::{marker::PhantomData, task::ready};

pub trait Packet: Any {}

pub trait ConnectivityFunction {
    fn send_connectivity(&mut self, packet: &WrappedPacket) -> SendConnectivity;
    fn pre_receive_connectivity(&mut self, dst: NodeId, port: u64) -> PreReceiveConnectivity;
    fn receive_connectivity(&mut self, packet: &WrappedPacket) -> ReceiveConnectivity;
}

pub struct WrappedPacket {
    pub src_port: u64,
    pub dst_port: u64,
    pub src: NodeId,
    pub dst: NodeId,
    pub id: PacketId,
    pub content: Box<dyn Packet>,
}

pub struct HalfPacket {
    port: u64,
    node: NodeId,
    content: Box<dyn Packet>,
}

impl WrappedPacket {
    pub fn content(&self) -> &dyn Packet {
        &*self.content
    }
    pub fn dst_port(&self) -> u64 {
        self.dst_port
    }
    pub fn src_port(&self) -> u64 {
        self.src_port
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
pub struct PacketId(pub u64);

pub enum SendConnectivity {
    Drop,
    Error { error: Error },
    Deliver { deliver_at: Instant },
}

pub enum PreReceiveConnectivity {
    /// Receive a packet if any, wait otherwise.
    Continue,
    /// Return an error.
    /// The queue of incoming packages is not modified.
    Error { error: Error },
}

pub enum ReceiveConnectivity {
    Receive,
    ErrorDiscard { error: Error },
    SilentDiscard,
}

pub struct PacketNetwork {
    connectivity: Box<dyn ConnectivityFunction>,
    receivers: HashMap<(NodeId, TypeId, u64), mpsc::UnboundedSender<WrappedPacket>>,
    pre_next_id: u64,
}

impl PacketNetwork {
    pub fn new(connectivity: impl ConnectivityFunction + 'static) -> Self {
        PacketNetwork {
            connectivity: Box::new(connectivity),
            receivers: HashMap::default(),
            pre_next_id: 0,
        }
    }

    fn enqueue_message(&mut self, packet: WrappedPacket) {
        let key = (
            packet.dst,
            packet_type_id(&*packet.content),
            packet.dst_port,
        );
        if let Some(r) = self.receivers.get_mut(&key) {
            r.send(packet);
        }
    }
}

impl Simulator for PacketNetwork {}

pub fn packet_type_id(p: &dyn Packet) -> TypeId {
    (*p).type_id()
}

pub struct NetworkBox(Box<dyn NetworkTrait>);

pub trait NetworkTrait: Simulator {
    fn open(&mut self, ty: TypeId, port: u64) -> Result<Box<dyn Socket>, Error>;
}

pub trait Socket:
    Stream<Item = Result<HalfPacket, Error>> + Sink<HalfPacket, Error = Error>
{
}

impl Socket for DefaultSocket {}

impl Sink<HalfPacket> for DefaultSocket {
    type Error = Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: HalfPacket) -> Result<(), Self::Error> {
        let this = self.project();
        debug_assert_eq!(packet_type_id(&*item.content), *this.ty);
        debug_assert_eq!(*this.node, NodeId::current());
        this.simulator.with(|net| {
            net.pre_next_id += 1;
            let msg = WrappedPacket {
                id: PacketId(net.pre_next_id),
                src: *this.node,
                src_port: *this.port,
                dst: item.node,
                dst_port: item.port,
                content: item.content,
            };
            match net.connectivity.send_connectivity(&msg) {
                SendConnectivity::Drop => Ok(()),
                SendConnectivity::Error { error } => Err(error),
                SendConnectivity::Deliver { deliver_at } => {
                    let handle = this.simulator.clone();
                    spawn(async move {
                        sleep_until(deliver_at).await;
                        handle.with(|net| net.enqueue_message(msg));
                    });
                    Ok(())
                }
            }
        })
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl Stream for DefaultSocket {
    type Item = Result<HalfPacket, Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        this.simulator.with(|net| {
            match net
                .connectivity
                .pre_receive_connectivity(*this.node, *this.port)
            {
                PreReceiveConnectivity::Continue => (),
                PreReceiveConnectivity::Error { error } => return Poll::Ready(Some(Err(error))),
            }
            loop {
                let msg = ready!(this.recv.as_mut().poll_next(cx)).unwrap();
                break Poll::Ready(Some(match net.connectivity.receive_connectivity(&msg) {
                    ReceiveConnectivity::Receive => Ok(HalfPacket {
                        port: msg.src_port,
                        node: msg.src,
                        content: msg.content,
                    }),
                    ReceiveConnectivity::ErrorDiscard { error } => Err(error),
                    ReceiveConnectivity::SilentDiscard => {
                        continue;
                    }
                }));
            }
        })
    }
}

impl Simulator for NetworkBox {
    fn create_node(&mut self) {
        self.0.create_node()
    }

    fn stop_node(&mut self) {
        self.0.stop_node()
    }

    fn start_node(&mut self) {
        self.0.start_node()
    }
}

impl NetworkTrait for PacketNetwork {
    fn open(&mut self, ty: TypeId, port: u64) -> Result<Box<dyn Socket>, Error> {
        let node = NodeId::current();
        let Entry::Vacant(x) = self.receivers.entry((node, ty, port)) else {
            return Err(Error::new(ErrorKind::AddrInUse, "address in use"));
        };
        let (s, recv) = mpsc::unbounded();
        x.insert(s);
        Ok(Box::new(DefaultSocket {
            port,
            ty,
            node,
            recv,
            simulator: simulator(),
        }))
    }
}

pin_project_lite::pin_project! {
    struct DefaultSocket {
        #[pin]
        recv: mpsc::UnboundedReceiver<WrappedPacket>,
        simulator: SimulatorHandle<PacketNetwork>,
        ty:TypeId,
        node:NodeId,
        port:u64,
    }

    impl PinnedDrop for DefaultSocket {
        fn drop(this:Pin<&mut Self>) {
            this.simulator.with(|net| {
                net.receivers
                    .remove(&(this.node, this.ty, this.port))
                    .unwrap();
            })
        }
    }
}

pub struct SocketBox<A, B> {
    inner: Pin<Box<dyn Socket>>,
    port: u64,
    dst: NodeId,
    _p: PhantomData<fn(A) -> B>,
}

impl<A: Packet, B: Packet> Stream for SocketBox<A, B> {
    type Item = Result<(NodeId, u64, B), Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx).map(|x| {
            Some(x.unwrap().map(|x| {
                (
                    x.node,
                    x.port,
                    *(x.content as Box<dyn Any>).downcast().unwrap(),
                )
            }))
        })
    }
}

impl<A: Packet, B: Packet> Sink<(NodeId, u64, A)> for SocketBox<A, B> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.inner.as_mut().poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: (NodeId, u64, A)) -> Result<(), Self::Error> {
        self.inner.as_mut().start_send(HalfPacket {
            node: item.0,
            port: item.1,
            content: Box::new(item.2),
        })
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.inner.as_mut().poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.inner.as_mut().poll_close(cx)
    }
}
