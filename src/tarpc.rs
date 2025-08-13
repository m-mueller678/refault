use std::{
    collections::{HashMap, hash_map::Entry},
    io::Error,
    marker::PhantomData,
    pin::Pin,
    task::{Poll, ready},
};

use futures::{Sink, SinkExt, Stream, StreamExt, stream};

use crate::{
    context::NodeId,
    packet_network::{Packet, SocketBox},
    simulator::{Simulator, simulator},
};

impl Simulator for Sim {}
struct Sim {
    next_id: u64,
}

impl Sim {
    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

struct WithId<T> {
    x: T,
    id: u64,
}

struct Request<A, B> {
    request: Option<A>,
    _p: PhantomData<B>,
}

struct Response<A, B> {
    response: Option<B>,
    _p: PhantomData<A>,
}

impl<A: 'static, B: 'static> Packet for Request<A, B> {}
impl<A: 'static, B: 'static> Packet for Response<A, B> {}

pub struct ServerConnection<A: 'static, B: 'static> {
    socket: SocketBox<Request<A, B>>,
    remote_node: NodeId,
    remote_port: u64,
}

async fn listen_next<A: 'static, B: 'static>(
    mut socket: &mut SocketBox<Request<A, B>>,
) -> Result<(NodeId, u64), Error> {
    let request = socket.next().await.unwrap()?;
    debug_assert!(request.2.request.is_none());
    Ok((request.0, request.1))
}

async fn listen<A: 'static, B: 'static>(
    port: u16,
) -> Result<impl Stream<Item = Result<ServerConnection<A, B>, Error>>, Error> {
    let socket = SocketBox::<Request<A, B>>::new(port as u64)?;
    let sim = simulator::<Sim>();
    Ok(stream::try_unfold(socket, move |mut socket| {
        let sim = sim.clone();
        async move {
            let (node, port) = listen_next(&mut socket).await?;
            let connection_id = sim.with(|x| x.next_id());
            let mut con_socket = SocketBox::new(port)?;
            con_socket
                .feed((
                    node,
                    port,
                    Response::<A, B> {
                        response: None,
                        _p: PhantomData,
                    },
                ))
                .await?;
            Ok(Some((
                (ServerConnection {
                    remote_node: node,
                    remote_port: port,
                    socket: con_socket,
                }),
                socket,
            )))
        }
    }))
}

impl<A: 'static, B: 'static> Stream for ServerConnection<A, B> {
    type Item = Result<A, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.socket
            .poll_next_unpin(cx)
            .map(|x| Some(x.unwrap().map(|x| x.2.request.unwrap())))
    }
}

impl<A: 'static, B: 'static> Sink<B> for ServerConnection<A, B> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_ready_unpin(&mut self.socket, cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: B) -> Result<(), Self::Error> {
        let item = (
            self.remote_node,
            self.remote_port,
            Response {
                response: Some(item),
                _p: PhantomData,
            },
        );
        SinkExt::<(NodeId, u64, Response<A, B>)>::start_send_unpin(&mut self.socket, item)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_flush_unpin(&mut self.socket, cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_close_unpin(&mut self.socket, cx)
    }
}

pub struct ClientConnection<A: 'static, B: 'static> {
    socket: SocketBox<Response<A, B>>,
    remote_port: u64,
    remote_node: NodeId,
}

impl<A: 'static, B: 'static> ClientConnection<A, B> {
    pub async fn new(node: NodeId, port: u16) -> Result<Self, Error> {
        let mut socket =
            SocketBox::<Response<A, B>>::new(simulator::<Sim>().with(|s| s.next_id()))?;
        socket
            .send((
                node,
                port as u64,
                Request::<A, B> {
                    request: None,
                    _p: PhantomData,
                },
            ))
            .await;
        let response = socket.next().await.unwrap()?;
        debug_assert!(response.2.response.is_none());
        debug_assert!(response.0 == node);
        debug_assert!(response.1 > u16::MAX as u64);
        Ok(ClientConnection {
            socket,
            remote_port: response.1,
            remote_node: node,
        })
    }
}

impl<A: 'static, B: 'static> Stream for ClientConnection<A, B> {
    type Item = Result<B, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Poll::Ready(Some(loop {
            break match ready!(self.socket.poll_next_unpin(cx)).unwrap() {
                Ok((
                    _,
                    _,
                    Response {
                        response: None,
                        _p: PhantomData,
                    },
                )) => continue,
                Ok((
                    _,
                    _,
                    Response {
                        response: Some(x),
                        _p: PhantomData,
                    },
                )) => Ok(x),
                Err(e) => Err(e),
            };
        }))
    }
}

impl<A: 'static, B: 'static> Sink<A> for ClientConnection<A, B> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_ready_unpin(&mut self.socket, cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: A) -> Result<(), Self::Error> {
        let item = (
            self.remote_node,
            self.remote_port,
            Request {
                request: Some(item),
                _p: PhantomData,
            },
        );
        SinkExt::<(NodeId, u64, Request<A, B>)>::start_send_unpin(&mut self.socket, item)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_flush_unpin(&mut self.socket, cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<(NodeId, u64, Response<A, B>)>::poll_close_unpin(&mut self.socket, cx)
    }
}
