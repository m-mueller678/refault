//! A simulated packet network.
//!
//! Packets have a source and a destination [Addr] consisting of of a [NodeId] and a port [Id].
//! Any type implementing [Packet] may be sent as a packet payload.
//! Packets are only received if a socket with matching port and type is opened on the destination node.
//! Different packets may specify different reliability requirements that will be upheld by the network.
//! A SendFunction may be used to customize network behaviour.
#![doc=concat!("```\n",include_str!("net/net_doc_example.rs"),"```\n`")]
use crate::{
    id::Id,
    node_id::NodeId,
    simulator::{Simulator, SimulatorHandle},
    time::sleep_until,
};
use either::Either::{self, Left};
use futures::future::ready;
use futures_intrusive::channel::LocalChannel;
use std::{
    any::{Any, type_name},
    collections::{HashMap, hash_map::Entry},
    fmt::Display,
    io::Error,
    pin::Pin,
    rc::Rc,
    time::Instant,
};
use std::{marker::PhantomData, time::Duration};
use typeid::ConstTypeId;

/// The payload of a simulated network packet.
pub trait Packet: Any {
    #[doc(hidden)]
    // This should not be overridden.
    fn packet_type_id(&self) -> ConstTypeId {
        ConstTypeId::of::<Self>()
    }

    /// Determines which network imperfections should be applied to the packet.
    ///
    /// See [ServiceLevel] for details.
    /// By default, packets will be delivered at most once and in arbitrary order.
    #[allow(unused_variables)]
    fn service_level(&self, src: Addr, dst: Addr) -> ServiceLevel {
        ServiceLevel {
            ordering: None,
            allow_drop: true,
            allow_multiple: false,
        }
    }

    /// Clone the packet.
    ///
    /// This is only invoked if the service level permits packet duplication.
    fn clone_packet(&self) -> Self
    where
        Self: Sized,
    {
        unimplemented!()
    }
}

/// Specifies which network imperfections should be applied to a packet.
pub struct ServiceLevel {
    /// If this is `None`, the packet may be arbitrarily reordered with respect to all other packets.
    /// Otherwise, the packet must not be reordered with respect to other packets with the same key.
    /// The packet may be arbitrarily reordered with respect to packets with different keys.
    pub ordering: Option<OrderingKey>,
    /// Specifies if packets may be dropped.
    ///
    /// Note that packets will still be dropped if at the time the packet is set to be delivered:
    ///   - there is no socket listening on the destination address
    ///   - the receive buffer of the listening socket is full
    pub allow_drop: bool,
    /// Specifies if packets may be delivered more than once.
    ///
    /// If this is `false`, the [PacketRc] must not be cloned.
    pub allow_multiple: bool,
}

/// Specifies how packets may be reordered.
///
/// Packets with equal ordering keys may not reordered with respect to each other.
/// Packets with different keys may be freely reordered.
/// Generally, the more information you add, the more reordering is permitted.
#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct OrderingKey {
    ty: Option<ConstTypeId>,
    dst_addr: Option<Addr>,
}

impl OrderingKey {
    /// Construct an empty ordering key.
    ///
    /// This key will be equal to all other keys returned from empty (unless more information is added).
    pub fn empty() -> Self {
        OrderingKey {
            ty: None,
            dst_addr: None,
        }
    }

    /// Add the receiver address.
    pub fn with_dst_addr(mut self, addr: Addr) -> Self {
        assert!(self.dst_addr.replace(addr).is_none());
        self
    }

    /// Add the type of the packet.
    pub fn with_type<P: Packet>(mut self) -> Self {
        assert!(self.ty.replace(ConstTypeId::of::<P>()).is_none());
        self
    }
}

/// An address in the simulated network.
#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Addr {
    pub node: NodeId,
    pub port: Id,
}

/// The function that is invoked to send a packet.
///
/// It is invoked by the network simulator in [send](Net::send) and related functions.
/// It is used to customize how the simulated network behaves.
/// Given a packet, it should invoke methods on the passed [Inboxes] object to cause the recipient to receive the packet.
/// A send function may choose to deliver the same packet multiple times, not deliver it at all, or deliver errors instead to simulate various network behaviours.
/// Some packets place restrictions on which of these behaviours are allowed.
/// These are specified in the [ServiceLevel] of the packet.
/// The send function must honour these restrictions.
///
/// An implementation that always delivers packets after a fixed delay is provided with [perfect_connectivity].
pub type SendFunction = Box<dyn FnMut(&mut Inboxes, PacketRc) -> SendFunctionOutput>;

/// The output of a [SendFunction].
#[doc(hidden)]
pub type SendFunctionOutput =
    Either<Result<(), Error>, Pin<Box<dyn Future<Output = Result<(), Error>>>>>;

/// Constructs a [SendFunction] that delivers packets exactly once after a fixed delay.
pub fn perfect_connectivity(latency: Duration) -> SendFunction {
    Box::new(move |queues, packet| {
        queues.enqueue_packet(Instant::now() + latency, packet);
        Left(Ok(()))
    })
}

/// Additional Information about a packet wrapped in a [PacketRc];
pub struct PacketHeader {
    src: Addr,
    dst: Addr,
    ty: ConstTypeId,
    service_level: ServiceLevel,
}

struct WrappedPacketImpl<T> {
    pub header: PacketHeader,
    packet: T,
}

#[derive(Clone)]
/// A type erased [Packet] along with source and destination address and [ServiceLevel].
///
/// The packet must not be cloned if the service level disallows packet duplicaiton.
pub struct PacketRc(Rc<dyn WrappedPacketTrait>);

impl PacketRc {
    /// Wrap a packet.
    pub fn new<T: Packet>(src: Addr, dst: Addr, packet: T) -> Self {
        PacketRc(Rc::new(WrappedPacketImpl {
            header: PacketHeader {
                src,
                dst,
                ty: packet.packet_type_id(),
                service_level: packet.service_level(src, dst),
            },
            packet,
        }))
    }
    /// Get the packet headder containing everything but the packet itself.
    pub fn header(&self) -> &PacketHeader {
        self.0.project().0
    }
    /// Get the contained packet.
    ///
    /// You should not call (service_level)[Packet::service_level] on this.
    /// It has already been computed and stored in the header.
    pub fn packet(&self) -> &dyn Packet {
        self.0.project().1
    }
}

trait WrappedPacketTrait: Any {
    fn project(&self) -> (&PacketHeader, &dyn Packet);
}

impl<T: Packet> WrappedPacketTrait for WrappedPacketImpl<T> {
    fn project(&self) -> (&PacketHeader, &dyn Packet) {
        (&self.header, &self.packet)
    }
}

impl PacketHeader {
    /// Get the source address.
    pub fn src(&self) -> Addr {
        self.src
    }
    /// Get the destination address.
    pub fn dst(&self) -> Addr {
        self.dst
    }
    /// Get the service level.
    pub fn service_level(&self) -> &ServiceLevel {
        &self.service_level
    }
}

/// The network simulator.
///
/// See the [module](self) level docs for more info.
pub struct Net {
    send_function: SendFunction,
    inboxes: Inboxes,
}

type Inbox = LocalChannel<Result<PacketRc, Error>, [Result<PacketRc, Error>; 8]>;

impl Simulator for Net {
    fn start_node(&mut self) {
        let node = NodeId::current();
        for (key, inbox) in &self.inboxes.0 {
            if key.0.node == node {
                while inbox.try_receive().is_ok() {}
            }
        }
    }

    fn create_node(&mut self) {
        self.start_node();
    }
}

/// A simulated network socket for sending and receiving packets.
///
/// Each Socket is associated with an [Addr].
/// It receives packets that match this [Addr] and the type of the Socket `t`.
///
/// Packets can also be sent directly via [Net::send] without constructing a socket.
pub struct Socket<T> {
    simulator: SimulatorHandle<Net>,
    inbox: Rc<Inbox>,
    local_addr: Addr,
    _p: PhantomData<fn(T) -> T>,
}

/// The future returned from [Socket::receive]
pub type SocketReceiveFuture<T: Packet> = impl Future<Output = Result<(T, Addr), Error>>;
/// The future returned from [Socket::send]
pub type SocketSendFuture = impl Future<Output = Result<(), Error>>;
/// The future returned from [Net::send]
pub type SendFuture = impl Future<Output = Result<(), Error>>;

#[derive(Debug)]
/// The error returned from [Socket::open].
pub struct AddrInUseError {
    addr: Addr,
    ty: &'static str,
}

impl Display for AddrInUseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let AddrInUseError { addr, ty } = self;
        write!(f, "socket exists for address {addr:?}, type {ty:?}")
    }
}

impl std::error::Error for AddrInUseError {}

impl From<AddrInUseError> for Error {
    fn from(value: AddrInUseError) -> Self {
        Error::new(std::io::ErrorKind::AddrInUse, value)
    }
}

impl<T: Packet> Socket<T> {
    /// Open a socket on the current node with the specified port.
    ///
    /// The socket will receive packets of type `T` that are addressed to this node and port.
    /// Returns an error if there is already a socket with this address.
    pub fn open(port: Id) -> Result<Self, AddrInUseError> {
        let addr = Addr {
            port,
            node: NodeId::current(),
        };
        let simulator = SimulatorHandle::<Net>::get();
        let ret =
            simulator.with(
                |net| match net.inboxes.0.entry((addr, ConstTypeId::of::<T>())) {
                    Entry::Occupied(_) => Err(AddrInUseError {
                        addr,
                        ty: type_name::<T>(),
                    }),
                    Entry::Vacant(x) => {
                        let inbox = Rc::new(Inbox::new());
                        x.insert(inbox.clone());
                        Ok(Socket {
                            inbox,
                            _p: PhantomData,
                            simulator: simulator.clone(),
                            local_addr: addr,
                        })
                    }
                },
            )?;
        Ok(ret)
    }

    /// Returns the port this node is bound to.
    pub fn local_port(&self) -> Id {
        self.local_addr.port
    }

    #[define_opaque(SocketSendFuture)]
    /// Send a packet from this socket.
    ///
    /// This is equivalent to invoking [Net::send] with the local address of this socket as src.
    pub fn send<U: Packet>(&self, dst: Addr, packet: U) -> SocketSendFuture {
        self.simulator
            .with(|net| net.send_wrapped(PacketRc::new(self.local_addr, dst, packet)))
    }

    /// Receive a packet.
    #[define_opaque(SocketReceiveFuture)]
    pub fn receive(&self) -> SocketReceiveFuture<T> {
        let inbox = self.inbox.clone();
        async move {
            inbox.receive().await.unwrap().map(|wrapped| {
                let wrapped = (wrapped.0 as Rc<dyn Any>)
                    .downcast::<WrappedPacketImpl<T>>()
                    .unwrap();
                let addr = wrapped.header.src;
                (
                    match Rc::try_unwrap(wrapped) {
                        Ok(x) => x.packet,
                        Err(x) => x.packet.clone_packet(),
                    },
                    addr,
                )
            })
        }
    }
}

impl<T> Drop for Socket<T> {
    fn drop(&mut self) {
        self.simulator.with(|net| {
            let removed = net
                .inboxes
                .0
                .remove(&(self.local_addr, ConstTypeId::of::<T>()));
            debug_assert!(removed.is_some_and(|x| Rc::ptr_eq(&x, &self.inbox)));
        })
    }
}

impl Net {
    /// Construct a network simulator.
    ///
    /// The behaviour of send and receive functions is governed by the given [SendFunction].
    pub fn new(send_function: SendFunction) -> Self {
        Self {
            send_function,
            inboxes: Inboxes(HashMap::new()),
        }
    }

    #[define_opaque(SendFuture)]
    fn send_wrapped(&mut self, packet: PacketRc) -> SendFuture {
        assert!(packet.header().src.node == NodeId::current());
        (self.send_function)(&mut self.inboxes, packet).map_left(ready)
    }

    /// Send a packet as if it was sent from a socket bound to `src` on the current node.
    ///
    /// Invokes the [SendFunction] the network was created with.
    /// This will usually cause the socket with the specified address to receive that packet.
    /// If there is no socket listening on the destination address, or its buffer is full, the packet will be dropped.
    pub fn send<T: Packet>(&mut self, src_port: Id, dst: Addr, packet: T) -> SendFuture {
        self.send_wrapped(PacketRc::new(
            Addr {
                port: src_port,
                node: NodeId::current(),
            },
            dst,
            packet,
        ))
    }

    /// Access the receiver inboxes.
    ///
    /// This should be used by by the [SendFunction] to cause receivers to receive packages or network errors.
    pub fn inboxes(&mut self) -> &mut Inboxes {
        &mut self.inboxes
    }
}

/// The inboxes of allr sockets.
///
/// This object can be used to cause sockets to receive errors or packets.
/// It is passed to the [SendFunction], but can also be obtained from [Net::inboxes].
pub struct Inboxes(HashMap<(Addr, ConstTypeId), Rc<Inbox>>);

impl Inboxes {
    /// Cause the packet to be receied by its destination address at the specified time.
    pub fn enqueue_packet(&mut self, at: Instant, msg: PacketRc) {
        let header = msg.header();
        let key = (header.dst, header.ty);
        Self::enqueue(at, key, Ok(msg));
    }

    /// Cause the destination address to receive an error at the specified time.
    pub fn enqueue_error(&mut self, at: Instant, dst: Addr, ty: ConstTypeId, error: Error) {
        Self::enqueue(at, (dst, ty), Err(error));
    }

    fn enqueue(at: Instant, key: (Addr, ConstTypeId), msg: Result<PacketRc, Error>) {
        NodeId::INIT.spawn(async move {
            sleep_until(at).await;
            SimulatorHandle::<Net>::get().with(|net| {
                if let Some(inbox) = net.inboxes.0.get(&key) {
                    inbox.try_send(msg).ok();
                }
            })
        });
    }
}
