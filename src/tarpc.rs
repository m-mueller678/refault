use crate::{
    packet_network::{Addr, Addressed, Packet, SocketBox},
    runtime::Id,
};
use futures::{Sink, SinkExt, Stream, StreamExt, stream};
use std::{
    io::Error,
    marker::PhantomData,
    pin::Pin,
    task::{Poll, ready},
};

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
    remote_addr: Addr,
}

async fn listen_next<A: 'static, B: 'static>(
    socket: &mut SocketBox<Request<A, B>>,
) -> Result<Addr, Error> {
    let request = socket.next().await.unwrap()?;
    debug_assert!(request.content.request.is_none());
    Ok(request.addr)
}

pub fn listen<A: 'static, B: 'static>(
    port: Id,
) -> Result<impl Stream<Item = Result<ServerConnection<A, B>, Error>>, Error> {
    let socket = SocketBox::<Request<A, B>>::new(port)?;
    Ok(stream::try_unfold(socket, |mut socket| async move {
        let addr = listen_next(&mut socket).await?;
        let mut con_socket = SocketBox::new(Id::new())?;
        con_socket
            .feed(Addressed {
                addr,
                content: Response::<A, B> {
                    response: None,
                    _p: PhantomData,
                },
            })
            .await?;
        Ok(Some((
            (ServerConnection {
                remote_addr: addr,
                socket: con_socket,
            }),
            socket,
        )))
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
            .map(|x| Some(x.unwrap().map(|x| x.content.request.unwrap())))
    }
}

impl<A: 'static, B: 'static> Sink<B> for ServerConnection<A, B> {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<Addressed<Response<A, B>>>::poll_ready_unpin(&mut self.socket, cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: B) -> Result<(), Self::Error> {
        let item = Addressed {
            addr: self.remote_addr,
            content: Response {
                response: Some(item),
                _p: PhantomData,
            },
        };
        SinkExt::<Addressed<Response<A, B>>>::start_send_unpin(&mut self.socket, item)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<Addressed<Response<A, B>>>::poll_flush_unpin(&mut self.socket, cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<Addressed<Response<A, B>>>::poll_close_unpin(&mut self.socket, cx)
    }
}

pub struct ClientConnection<A: 'static, B: 'static> {
    socket: SocketBox<Response<A, B>>,
    remote_addr: Addr,
}

impl<A: 'static, B: 'static> ClientConnection<A, B> {
    pub async fn new(remote_addr: Addr) -> Result<Self, Error> {
        let mut socket = SocketBox::<Response<A, B>>::new(Id::new())?;
        socket
            .send(Addressed {
                addr: remote_addr,
                content: Request::<A, B> {
                    request: None,
                    _p: PhantomData,
                },
            })
            .await?;
        let response = socket.next().await.unwrap()?;
        debug_assert!(response.content.response.is_none());
        debug_assert!(response.addr.node == remote_addr.node);
        Ok(ClientConnection {
            socket,
            remote_addr: response.addr,
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
                Ok(Addressed {
                    addr: _,
                    content:
                        Response {
                            response: None,
                            _p: PhantomData,
                        },
                }) => continue,
                Ok(Addressed {
                    addr: _,
                    content:
                        Response {
                            response: Some(x),
                            _p: PhantomData,
                        },
                }) => Ok(x),
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
        SinkExt::<Addressed<Request<A, B>>>::poll_ready_unpin(&mut self.socket, cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: A) -> Result<(), Self::Error> {
        let item = Addressed {
            addr: self.remote_addr,
            content: Request {
                request: Some(item),
                _p: PhantomData,
            },
        };
        SinkExt::<Addressed<Request<A, B>>>::start_send_unpin(&mut self.socket, item)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<Addressed<Request<A, B>>>::poll_flush_unpin(&mut self.socket, cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        SinkExt::<Addressed<Request<A, B>>>::poll_close_unpin(&mut self.socket, cx)
    }
}
