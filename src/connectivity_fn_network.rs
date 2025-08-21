use crate::{
    context::executor::NodeId,
    packet_network::{Addr, Addressed, Packet, packet_type_id},
    simulator::{Simulator, SimulatorHandle, simulator},
    time::sleep_until,
};
use futures::FutureExt;
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

pub struct PerfectConnectivity(Duration);

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

struct ConNet {
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

struct ConNetSocket {
    simulator: SimulatorHandle<ConNet>,
    local_addr: Addr,
}

impl ConNetSocket {
    pub fn send<T: Packet>(
        &self,
        packet: Addressed<T>,
    ) -> impl Future<Output = Result<(), Error>> + use<T> {
        self.simulator.with(|net| {
            net.send(WrappedPacket {
                src: self.local_addr,
                dst: packet.addr,
                content: Box::new(packet.content),
            })
        })
    }

    pub fn receive<T: Packet>(&self) -> impl Future<Output = Result<Addressed<T>, Error>> + use<T> {
        self.simulator.with(|net| net.receive(self.local_addr))
    }
}

impl ConNet {
    pub fn send(
        &mut self,
        packet: WrappedPacket,
    ) -> impl Future<Output = Result<(), Error>> + use<> {
        assert!(packet.src.node == NodeId::current());
        (self.send_function)(packet)
    }

    pub fn receive<T: Packet>(
        &mut self,
        address: Addr,
    ) -> impl Future<Output = Result<Addressed<T>, Error>> + use<T> {
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
    ) -> impl Future<Output = Result<Addressed, Error>> + use<> {
        let inbox = self.with_queue((address, ty), |inbox| inbox.clone());
        async move { inbox.receive().await.unwrap() }
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
