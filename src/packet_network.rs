use crate::{
    context::executor::NodeId,
    runtime::Id,
    simulator::{Simulator, SimulatorHandle, simulator},
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

pub trait Packet: Any {
    fn packet_type_id(&self) -> ConstTypeId {
        ConstTypeId::of::<Self>()
    }

    fn service_level(&self, _src: Addr, _dst: Addr) -> ServiceLevel {
        ServiceLevel {
            ordering: None,
            allow_drop: true,
            allow_multiple: false,
        }
    }

    fn clone_packet(&self) -> Self
    where
        Self: Sized,
    {
        unimplemented!()
    }
}

pub struct ServiceLevel {
    pub ordering: Option<OrderingKey>,
    pub allow_drop: bool,
    pub allow_multiple: bool,
}

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct OrderingKey {
    ty: Option<ConstTypeId>,
    dst_addr: Option<Addr>,
}

impl OrderingKey {
    pub fn empty() -> Self {
        OrderingKey {
            ty: None,
            dst_addr: None,
        }
    }

    pub fn with_dst_addr(mut self, addr: Addr) -> Self {
        assert!(self.dst_addr.replace(addr).is_none());
        self
    }

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

/// A network packet along with an address.
///
/// When returned from a receiving function, the address refers to the sender.
/// When passed into a transmitting function, it refers to the recipient.
pub struct Addressed<T> {
    pub addr: Addr,
    pub content: T,
}

impl<T> Addressed<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Addressed<U> {
        Addressed {
            addr: self.addr,
            content: f(self.content),
        }
    }
}

/// The function that is invoked to send a packet.
///
/// This function is used to customize how the simulated network behaves.
/// Given a packet, it should invoke methods on the passed `Receivers` handle to cause the recipient to receive the packet.
/// An implementation that always delivers packets after a fixed delay is provided with [perfect_connectivity].
/// A send function may choose to deliver the same packet multiple times, not deliver it at all, or deliver errors instead to simulate various network behaviours.
///
/// This is invoked by the network simulator in [send][ConNet::send].
/// Attempting to access the simmulator from within this call will therefore panic.
/// Instead, access the simulator from the returned future.
pub type SendFunction = Box<dyn FnMut(&mut ConNetQueues, WrappedPacket) -> SendFunctionOutput>;
pub type SendFunctionOutput =
    Either<Result<(), Error>, Pin<Box<dyn Future<Output = Result<(), Error>>>>>;

pub fn perfect_connectivity(latency: Duration) -> SendFunction {
    Box::new(move |queues, packet| {
        queues.enqueue_packet(Instant::now() + latency, packet);
        Left(Ok(()))
    })
}

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
pub struct WrappedPacket(Rc<dyn WrappedPacketPrivate>);

impl WrappedPacket {
    pub fn new<T: Packet>(src: Addr, dst: Addr, packet: T) -> Self {
        WrappedPacket(Rc::new(WrappedPacketImpl {
            header: PacketHeader {
                src,
                dst,
                ty: packet.packet_type_id(),
                service_level: packet.service_level(src, dst),
            },
            packet,
        }))
    }
    pub fn header(&self) -> &PacketHeader {
        self.0.project().0
    }
    pub fn packet(&self) -> &dyn Packet {
        self.0.project().1
    }
}

trait WrappedPacketPrivate: Any {
    fn project(&self) -> (&PacketHeader, &dyn Packet);
}

impl<T: Packet> WrappedPacketPrivate for WrappedPacketImpl<T> {
    fn project(&self) -> (&PacketHeader, &dyn Packet) {
        (&self.header, &self.packet)
    }
}

impl PacketHeader {
    pub fn src(&self) -> Addr {
        self.src
    }
    pub fn dst(&self) -> Addr {
        self.dst
    }
    pub fn service_level(&self) -> &ServiceLevel {
        &self.service_level
    }
}

pub struct ConNet {
    send_function: SendFunction,
    queues: ConNetQueues,
}

type Inbox = LocalChannel<Result<WrappedPacket, Error>, [Result<WrappedPacket, Error>; 8]>;

impl Simulator for ConNet {
    fn start_node(&mut self) {
        let node = NodeId::current();
        for (key, inbox) in &self.queues.receivers {
            if key.0.node == node {
                while inbox.try_receive().is_ok() {}
            }
        }
    }

    fn create_node(&mut self) {
        self.start_node();
    }
}

pub struct ConNetSocket<T> {
    simulator: SimulatorHandle<ConNet>,
    inbox: Rc<Inbox>,
    local_addr: Addr,
    _p: PhantomData<fn(T) -> T>,
}

pub type SocketReceiveFuture<T: Packet> = impl Future<Output = Result<Addressed<T>, Error>>;
pub type SendFuture = impl Future<Output = Result<(), Error>>;
pub type SocketSendFuture = impl Future<Output = Result<(), Error>>;

#[derive(Debug)]
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

impl<T: Packet> ConNetSocket<T> {
    pub fn open(port: Id) -> Result<Self, AddrInUseError> {
        let addr = Addr {
            port,
            node: NodeId::current(),
        };
        let simulator = simulator::<ConNet>();
        let ret = simulator.with(|net| {
            match net.queues.receivers.entry((addr, ConstTypeId::of::<T>())) {
                Entry::Occupied(_) => Err(AddrInUseError {
                    addr,
                    ty: type_name::<T>(),
                }),
                Entry::Vacant(x) => {
                    let inbox = Rc::new(Inbox::new());
                    x.insert(inbox.clone());
                    Ok(ConNetSocket {
                        inbox,
                        _p: PhantomData,
                        simulator: simulator.clone(),
                        local_addr: addr,
                    })
                }
            }
        })?;
        Ok(ret)
    }

    pub fn local_addr(&self) -> Addr {
        self.local_addr
    }

    pub fn local_port(&self) -> Id {
        self.local_addr.port
    }

    pub fn send(&self, packet: Addressed<T>) -> SocketSendFuture {
        self.send_any(packet)
    }

    #[define_opaque(SocketSendFuture)]
    pub fn send_any<U: Packet>(&self, packet: Addressed<U>) -> SocketSendFuture {
        self.simulator.with(|net| {
            net.send_wrapped(WrappedPacket::new(
                self.local_addr,
                packet.addr,
                packet.content,
            ))
        })
    }

    #[define_opaque(SocketReceiveFuture)]
    pub fn receive(&self) -> SocketReceiveFuture<T> {
        let inbox = self.inbox.clone();
        async move {
            inbox.receive().await.unwrap().map(|wrapped| {
                let wrapped = (wrapped.0 as Rc<dyn Any>)
                    .downcast::<WrappedPacketImpl<T>>()
                    .unwrap();
                Addressed {
                    addr: wrapped.header.src,
                    content: match Rc::try_unwrap(wrapped) {
                        Ok(x) => x.packet,
                        Err(x) => x.packet.clone_packet(),
                    },
                }
            })
        }
    }
}

impl<T> Drop for ConNetSocket<T> {
    fn drop(&mut self) {
        self.simulator.with(|net| {
            let removed = net
                .queues
                .receivers
                .remove(&(self.local_addr, ConstTypeId::of::<T>()));
            debug_assert!(removed.is_some_and(|x| Rc::ptr_eq(&x, &self.inbox)));
        })
    }
}

impl ConNet {
    pub fn new(send_function: SendFunction) -> Self {
        Self {
            send_function,
            queues: ConNetQueues {
                receivers: HashMap::new(),
            },
        }
    }

    #[define_opaque(SendFuture)]
    pub fn send_wrapped(&mut self, packet: WrappedPacket) -> SendFuture {
        assert!(packet.header().src.node == NodeId::current());
        (self.send_function)(&mut self.queues, packet).map_left(ready)
    }

    pub fn send<T: Packet>(&mut self, src: Addr, dst: Addr, packet: T) -> SendFuture {
        self.send_wrapped(WrappedPacket::new(src, dst, packet))
    }

    pub fn queues(&mut self) -> &mut ConNetQueues {
        &mut self.queues
    }
}

pub struct ConNetQueues {
    receivers: HashMap<(Addr, ConstTypeId), Rc<Inbox>>,
}

impl ConNetQueues {
    /// Cause the destination node to receive the packet at the specified time.
    pub fn enqueue_packet(&mut self, at: Instant, msg: WrappedPacket) {
        let header = msg.header();
        let key = (header.dst, header.ty);
        Self::enqueue(at, key, Ok(msg));
    }

    /// Cause the destination node to receive an error at the specified time.
    pub fn enqueue_error(&mut self, at: Instant, dst: Addr, ty: ConstTypeId, error: Error) {
        Self::enqueue(at, (dst, ty), Err(error));
    }

    fn enqueue(at: Instant, key: (Addr, ConstTypeId), msg: Result<WrappedPacket, Error>) {
        NodeId::INIT.spawn(async move {
            sleep_until(at).await;
            simulator::<ConNet>().with(|net| {
                if let Some(inbox) = net.queues.receivers.get(&key) {
                    inbox.try_send(msg).ok();
                }
            })
        });
    }
}
