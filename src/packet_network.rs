use crate::{
    context::executor::NodeId,
    fragile_future::FragileFuture,
    runtime::Id,
    simulator::{Simulator, SimulatorHandle, simulator},
    time::sleep_until,
};
use futures::{FutureExt, never::Never};
use futures_intrusive::channel::UnbufferedChannel;
use std::time::Duration;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io::Error,
    pin::Pin,
    rc::Rc,
    time::Instant,
};

pub trait Packet: Any {}
impl Packet for () {}
impl Packet for Never {}

/// An address in the simulated network.
#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy)]
pub struct Addr {
    pub node: NodeId,
    pub port: Id,
}

/// A network packet along with an address.
///
/// When returned from a receiving function, the address refers to the sender.
/// When passed into a transmitting function, it refers to the recipient.
pub struct Addressed<T = Box<dyn Packet>> {
    pub addr: Addr,
    pub content: T,
}

/// Get the TypeId of a packet.
///
/// Using this prevents you from accidentally ending up with the TypeId of `Box<dyn Packet>` rather than the id of the contained value.
pub fn packet_type_id(p: &dyn Packet) -> TypeId {
    (*p).type_id()
}

/// The function that is invoked to send a packet.
///
/// This function is used to customize how the simulated network behaves.
/// Given a packet, it should invoke methods on the passed `Receivers` handle to cause the recipient to receive the packet.
/// An implementation that always delivers packets after a fixed delay is provided with [perfecet_connectivity].
/// A send function may choose to deliver the same packet multiple times, not deliver it at all, or deliver errors instead to simulate various network behaviours.
///
/// This is invoked by the network simulator in [send][ConNet::send].
/// Attempting to access the simmulator from within this call will therefore panic.
/// Instead, access the simulator from the returned future.
pub type SendFunction =
    Box<dyn FnMut(WrappedPacket) -> Pin<Box<dyn Future<Output = Result<(), Error>>>>>;

pub fn perfect_connectivity(latency: Duration) -> SendFunction {
    Box::new(move |packet| {
        Box::pin(async move {
            ConNet::enqueue_packet(Instant::now() + latency, packet);
            Ok(())
        })
    })
}

/// A Packet with sender and recipient address
///
/// This is passed to a [SendFunction].
pub struct WrappedPacket {
    pub src: Addr,
    pub dst: Addr,
    pub content: Box<dyn Packet>,
}

pub struct ConNet {
    send_function: SendFunction,
    receivers: HashMap<(Addr, TypeId), Rc<Inbox>>,
}

type Inbox = UnbufferedChannel<Result<Addressed, Error>>;

impl Simulator for ConNet {
    fn start_node(&mut self) {
        let node = NodeId::current();
        for (key, inbox) in &self.receivers {
            if key.0.node == node {
                while inbox.try_receive().is_ok() {}
            }
        }
    }

    fn create_node(&mut self) {
        self.start_node();
    }
}

pub struct ConNetSocket {
    simulator: SimulatorHandle<ConNet>,
    local_addr: Addr,
}

pub type ReceiveFuture<T: Packet> = impl Future<Output = Result<Addressed<T>, Error>> + Send;
pub type SocketReceiveFuture<T: Packet> = impl Future<Output = Result<Addressed<T>, Error>> + Send;
pub type SendFuture = impl Future<Output = Result<(), Error>> + Send;
pub type SocketSendFuture = impl Future<Output = Result<(), Error>> + Send;

impl ConNetSocket {
    pub fn open(port: Id) -> Self {
        ConNetSocket {
            simulator: simulator(),
            local_addr: Addr {
                port,
                node: NodeId::current(),
            },
        }
    }

    #[define_opaque(SocketSendFuture)]
    pub fn send<T: Packet>(&self, packet: Addressed<T>) -> SocketSendFuture {
        self.simulator.with(|net| {
            net.send(WrappedPacket {
                src: self.local_addr,
                dst: packet.addr,
                content: Box::new(packet.content),
            })
        })
    }

    #[define_opaque(SocketReceiveFuture)]
    pub fn receive<T: Packet>(&self) -> SocketReceiveFuture<T> {
        FragileFuture::new(self.simulator.with(|net| net.receive(self.local_addr)))
    }
}

impl ConNet {
    #[define_opaque(SendFuture)]
    pub fn send(&mut self, packet: WrappedPacket) -> SendFuture {
        assert!(packet.src.node == NodeId::current());
        FragileFuture::new((self.send_function)(packet))
    }

    #[define_opaque(ReceiveFuture)]
    pub fn receive<T: Packet>(&mut self, address: Addr) -> ReceiveFuture<T> {
        assert!(address.node == NodeId::current());
        self.receive_any(TypeId::of::<T>(), address).map(|result| {
            result.map(|packet| Addressed {
                content: *(packet.content as Box<dyn Any>).downcast().unwrap(),
                addr: packet.addr,
            })
        })
    }

    fn receive_any(
        &mut self,
        ty: TypeId,
        address: Addr,
    ) -> impl Future<Output = Result<Addressed, Error>> + Send + use<> {
        let inbox = self.with_queue((address, ty), |inbox| inbox.clone());
        FragileFuture::new(async move { inbox.receive().await.unwrap() })
    }

    /// Cause the destination node to receive the packet at the specified time.
    pub fn enqueue_packet(at: Instant, msg: WrappedPacket) {
        let key = (msg.dst, packet_type_id(&*msg.content));
        Self::enqueue(at, key, Ok(msg));
    }

    /// Cause the destination node to receive an error at the specified time.
    pub fn enqueue_error(at: Instant, dst: Addr, ty: TypeId, error: Error) {
        Self::enqueue(at, (dst, ty), Err(error));
    }

    fn enqueue(at: Instant, key: (Addr, TypeId), msg: Result<WrappedPacket, Error>) {
        NodeId::INIT
            .spawn(async move {
                sleep_until(at).await;
                let addressed = msg.map(|msg| Addressed {
                    addr: msg.src,
                    content: msg.content,
                });
                simulator::<ConNet>().with(|net| {
                    net.with_queue(key, |inbox| {
                        inbox.try_send(addressed).ok();
                    });
                })
            })
            .detach();
    }

    fn with_queue<R>(&mut self, key: (Addr, TypeId), f: impl FnOnce(&Rc<Inbox>) -> R) -> R {
        f(self
            .receivers
            .entry(key)
            .or_insert_with(|| Rc::new(Inbox::new())))
    }
}
