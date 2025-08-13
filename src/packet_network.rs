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

pub struct HalfPacket {
    pub port: Id,
    pub node: NodeId,
    pub content: Box<dyn Packet>,
}

pub fn packet_type_id(p: &dyn Packet) -> TypeId {
    (*p).type_id()
}

pub struct NetworkBox(Box<dyn NetworkTrait>);

impl NetworkBox {
    pub fn new<T: NetworkTrait>(inner: T) -> Self {
        NetworkBox(Box::new(inner))
    }
}

pub trait NetworkTrait: Simulator {
    fn open(&mut self, ty: TypeId, port: Id) -> Result<Pin<Box<dyn Socket>>, Error>;
}

pub trait Socket:
    Stream<Item = Result<HalfPacket, Error>> + Sink<HalfPacket, Error = Error>
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

pub struct SocketBox<Receive: Packet> {
    inner: Pin<Box<dyn Socket>>,
    _p: PhantomData<fn() -> Receive>,
}

impl<Receive: Packet> Stream for SocketBox<Receive> {
    type Item = Result<(NodeId, Id, Receive), Error>;

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

impl<Receive: Packet, A: Packet> Sink<(NodeId, Id, A)> for SocketBox<Receive> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.inner.as_mut().poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: (NodeId, Id, A)) -> Result<(), Self::Error> {
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

impl<Receive: Packet> SocketBox<Receive> {
    pub fn new(port: Id) -> Result<Self, Error> {
        simulator::<NetworkBox>().with(|net| {
            net.0
                .open(TypeId::of::<Receive>(), port)
                .map(|inner| SocketBox {
                    inner,
                    _p: PhantomData,
                })
        })
    }
}
