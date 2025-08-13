use crate::{
    context::NodeId,
    runtime::Id,
    simulator::{Simulator, simulator},
};
use futures::{Sink, Stream, never::Never};
use std::marker::PhantomData;
use std::{
    any::{Any, TypeId},
    io::Error,
    pin::Pin,
    task::Poll,
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

pub struct NetworkBox(Box<dyn NetworkBackend>);

impl NetworkBox {
    pub fn new<T: NetworkBackend>(inner: T) -> Self {
        NetworkBox(Box::new(inner))
    }
}

pub trait NetworkBackend: Simulator {
    fn open(&mut self, ty: TypeId, port: Id) -> Result<Pin<Box<dyn BackendSocket>>, Error>;
}

pub trait BackendSocket:
    Stream<Item = Result<Addressed, Error>> + Sink<Addressed, Error = Error>
{
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

pub struct Socket<Receive: Packet> {
    inner: Pin<Box<dyn BackendSocket>>,
    _p: PhantomData<fn() -> Receive>,
}

impl<Receive: Packet> Stream for Socket<Receive> {
    type Item = Result<Addressed<Receive>, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx).map(|x| {
            Some(x.unwrap().map(|x| Addressed {
                addr: x.addr,
                content: *(x.content as Box<dyn Any>).downcast().unwrap(),
            }))
        })
    }
}

impl<Receive: Packet, A: Packet> Sink<Addressed<A>> for Socket<Receive> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.inner.as_mut().poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Addressed<A>) -> Result<(), Self::Error> {
        self.inner.as_mut().start_send(Addressed {
            addr: item.addr,
            content: Box::new(item.content),
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

impl<Receive: Packet> Socket<Receive> {
    pub fn new(port: Id) -> Result<Self, Error> {
        simulator::<NetworkBox>().with(|net| {
            net.0
                .open(TypeId::of::<Receive>(), port)
                .map(|inner| Socket {
                    inner,
                    _p: PhantomData,
                })
        })
    }
}
