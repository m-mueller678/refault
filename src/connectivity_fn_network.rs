use crate::{
    context::NodeId,
    packet_network::{
        Addr, Addressed, BackendSocket, NetworkBackend, NetworkBox, Packet, packet_type_id,
    },
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
    fn pre_receive_connectivity(&mut self, dst: Addr) -> PreReceiveConnectivity;
    fn receive_connectivity(&mut self, packet: &WrappedPacket) -> ReceiveConnectivity;
}

/// A Packet with sender and recipient address and a unique packet id.
///
/// This is passed to a [ConnectivityFunction] for inspection.
pub struct WrappedPacket {
    src: Addr,
    dst: Addr,
    id: Id,
    content: Box<dyn Packet>,
}

impl WrappedPacket {
    pub fn content(&self) -> &dyn Packet {
        &*self.content
    }
    pub fn dst(&self) -> &Addr {
        &self.dst
    }
    pub fn src(&self) -> &Addr {
        &self.src
    }
    pub fn id(&self) -> Id {
        self.id
    }
}

/// See [ConnectivityFunction::send_connectivity].
pub enum SendConnectivity {
    Drop,
    Error { error: Error },
    Deliver { deliver_at: Instant },
}

/// See [ConnectivityFunction::pre_receive_connectivity].
pub enum PreReceiveConnectivity {
    /// Receive a packet if any, wait otherwise.
    Continue,
    /// Return an error.
    /// The queue of incoming packages is not modified.
    Error { error: Error },
}

/// See [ConnectivityFunction::receive_connectivity].
pub enum ReceiveConnectivity {
    Receive,
    ErrorDiscard { error: Error },
    SilentDiscard,
}

struct ConNet<C: ConnectivityFunction> {
    connectivity: C,
    receivers: HashMap<(Addr, TypeId), mpsc::UnboundedSender<WrappedPacket>>,
}

/// Create a network based on the connectivity function.
pub fn network_from_connectivity(connectivity: impl ConnectivityFunction + 'static) -> NetworkBox {
    NetworkBox::new(ConNet {
        connectivity,
        receivers: HashMap::default(),
    })
}

impl<T: ConnectivityFunction> Simulator for ConNet<T> {}
impl<C: ConnectivityFunction> BackendSocket for ConNetSocket<C> {}

impl<C: ConnectivityFunction> Sink<Addressed> for ConNetSocket<C> {
    type Error = Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Addressed) -> Result<(), Self::Error> {
        let this = self.project();
        debug_assert_eq!(packet_type_id(&*item.content), *this.ty);
        debug_assert_eq!(this.local_addr.node, NodeId::current());
        this.simulator.with(|net| {
            let msg = WrappedPacket {
                id: Id::new(),
                src: *this.local_addr,
                dst: item.addr,
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
                            let key = (msg.dst, packet_type_id(&*msg.content));
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

impl<C: ConnectivityFunction> Stream for ConNetSocket<C> {
    type Item = Result<Addressed, Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        this.simulator.with(|net| {
            match net.connectivity.pre_receive_connectivity(*this.local_addr) {
                PreReceiveConnectivity::Continue => (),
                PreReceiveConnectivity::Error { error } => return Poll::Ready(Some(Err(error))),
            }
            loop {
                let msg = ready!(this.recv.as_mut().poll_next(cx)).unwrap();
                break Poll::Ready(Some(match net.connectivity.receive_connectivity(&msg) {
                    ReceiveConnectivity::Receive => Ok(Addressed {
                        addr: msg.src,
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

impl<C: ConnectivityFunction> NetworkBackend for ConNet<C> {
    fn open(&mut self, ty: TypeId, port: Id) -> Result<Pin<Box<dyn BackendSocket>>, Error> {
        let local_addr = Addr {
            port,
            node: NodeId::current(),
        };
        let Entry::Vacant(x) = self.receivers.entry((local_addr, ty)) else {
            return Err(Error::new(ErrorKind::AddrInUse, "address in use"));
        };
        let (s, recv) = mpsc::unbounded();
        x.insert(s);
        Ok(Box::pin(ConNetSocket::<C> {
            local_addr,
            ty,
            recv,
            simulator: simulator(),
        }))
    }
}

pin_project_lite::pin_project! {
    struct ConNetSocket<C: ConnectivityFunction> {
        #[pin]
        recv: mpsc::UnboundedReceiver<WrappedPacket>,
        simulator: SimulatorHandle<ConNet<C>>,
        ty: TypeId,
        local_addr: Addr,
    }

    impl<C: ConnectivityFunction> PinnedDrop for ConNetSocket<C> {
        fn drop(this: Pin<&mut Self>) {
            this.simulator.with(|net| {
                net.receivers.remove(&(this.local_addr, this.ty)).unwrap();
            })
        }
    }
}
