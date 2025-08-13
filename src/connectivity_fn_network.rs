use crate::{
    context::NodeId,
    packet_network::{HalfPacket, NetworkBox, NetworkTrait, Packet, Socket, packet_type_id},
    runtime::{Id, spawn},
    simulator::{Simulator, SimulatorHandle, simulator},
    time::sleep_until,
};
use futures::{Sink, Stream};
use futures_channel::mpsc;
use std::task::ready;
use std::{
    any::TypeId,
    collections::{HashMap, hash_map::Entry},
    io::{Error, ErrorKind},
    pin::Pin,
    task::Poll,
    time::Instant,
};

pub trait ConnectivityFunction: 'static {
    fn send_connectivity(&mut self, packet: &WrappedPacket) -> SendConnectivity;
    fn pre_receive_connectivity(&mut self, dst: NodeId, port: Id) -> PreReceiveConnectivity;
    fn receive_connectivity(&mut self, packet: &WrappedPacket) -> ReceiveConnectivity;
}

pub struct WrappedPacket {
    pub src_port: Id,
    pub dst_port: Id,
    pub src: NodeId,
    pub dst: NodeId,
    pub id: Id,
    pub content: Box<dyn Packet>,
}

impl WrappedPacket {
    pub fn content(&self) -> &dyn Packet {
        &*self.content
    }
    pub fn dst_port(&self) -> Id {
        self.dst_port
    }
    pub fn src_port(&self) -> Id {
        self.src_port
    }
    pub fn src(&self) -> NodeId {
        self.src
    }
    pub fn dst(&self) -> NodeId {
        self.dst
    }
    pub fn id(&self) -> Id {
        self.id
    }
}

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

pub struct PacketNetwork<C: ConnectivityFunction> {
    connectivity: C,
    receivers: HashMap<(NodeId, TypeId, Id), mpsc::UnboundedSender<WrappedPacket>>,
}

pub fn network_from_connectivity(connectivity: impl ConnectivityFunction + 'static) -> NetworkBox {
    NetworkBox::new(PacketNetwork {
        connectivity,
        receivers: HashMap::default(),
    })
}

impl<T: ConnectivityFunction> Simulator for PacketNetwork<T> {}
impl<C: ConnectivityFunction> Socket for DefaultSocket<C> {}

impl<C: ConnectivityFunction> Sink<HalfPacket> for DefaultSocket<C> {
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
            let msg = WrappedPacket {
                id: Id::new(),
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
                        handle.with(|net| {
                            let key = (msg.dst, packet_type_id(&*msg.content), msg.dst_port);
                            if let Some(r) = net.receivers.get_mut(&key) {
                                r.unbounded_send(msg).unwrap();
                            }
                        });
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

impl<C: ConnectivityFunction> Stream for DefaultSocket<C> {
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

impl<C: ConnectivityFunction> NetworkTrait for PacketNetwork<C> {
    fn open(&mut self, ty: TypeId, port: Id) -> Result<Pin<Box<dyn Socket>>, Error> {
        let node = NodeId::current();
        let Entry::Vacant(x) = self.receivers.entry((node, ty, port)) else {
            return Err(Error::new(ErrorKind::AddrInUse, "address in use"));
        };
        let (s, recv) = mpsc::unbounded();
        x.insert(s);
        Ok(Box::pin(DefaultSocket::<C> {
            port,
            ty,
            node,
            recv,
            simulator: simulator(),
        }))
    }
}

pin_project_lite::pin_project! {
    struct DefaultSocket <C: ConnectivityFunction>{
        #[pin]
        recv: mpsc::UnboundedReceiver<WrappedPacket>,
        simulator: SimulatorHandle<PacketNetwork<C>>,
        ty:TypeId,
        node:NodeId,
        port:Id,
    }

    impl<C:ConnectivityFunction> PinnedDrop for DefaultSocket <C>{
        fn drop(this:Pin<&mut Self>) {
            this.simulator.with(|net| {
                net.receivers
                    .remove(&(this.node, this.ty, this.port))
                    .unwrap();
            })
        }
    }
}
