use crate::{
    agnostic_lite_runtime::SimRuntime,
    check_send::{CheckSend, Constraint, NodeBound},
    ip_addr::{IpAddrSimulator, TcpPortAssignment, resolve_socket_addrs},
    packet_network::{
        Addr, Addressed, ConNet, ConNetSocket, Packet, SendFuture, SocketReceiveFuture,
        WrappedPacket,
    },
    runtime::{Id, NodeId},
    simulator::{SimulatorHandle, simulator},
};
use agnostic_net::runtime::RuntimeLite;
use bytes::Bytes;
use futures::{AsyncRead, TryFutureExt, io::AsyncWrite};
use std::{
    cell::Cell,
    convert::identity,
    fmt::{Debug, Display},
    future::poll_fn,
    io::{Error, ErrorKind},
    net::SocketAddr,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, ready},
};

type Result<T, E = Error> = std::result::Result<T, E>;

impl Packet for TcpDatagram {}

pub struct TcpIncomingConnection {
    pub(crate) client_ip: SocketAddr,
    pub(crate) client_sim: Addr,
}

pub(super) enum TcpDatagram {
    Connect {
        client: SocketAddr,
        server: SocketAddr,
    },
    Refused,
    Accept,
    Data(Bytes),
}

pub struct TcpStream {
    pub inner: CheckSend<TcpStreamUnSend, NodeBound>,
}

pub struct TcpStreamUnSend {
    write: OwnedWriteHalfUnsend,
    read: OwnedReadHalfUnsend,
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner.write).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner.write).poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner.write).poll_close(cx)
    }
}

impl futures::io::AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner.read).poll_read(cx, buf)
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        unimplemented!()
    }
}

impl AsFd for TcpStream {
    fn as_fd(&self) -> std::os::unix::prelude::BorrowedFd<'_> {
        unimplemented!()
    }
}

pub struct ReuniteError {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
}

impl core::error::Error for ReuniteError {}

impl Display for ReuniteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("socket halves do not belong to same socket")
    }
}
impl Debug for ReuniteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("socket halves do not belong to same socket")
    }
}

impl agnostic_net::ReuniteError<TcpStream> for ReuniteError {
    fn into_components(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        (self.read, self.write)
    }
}

impl TryFrom<std::net::TcpStream> for TcpStream {
    type Error = Error;

    fn try_from(_: std::net::TcpStream) -> Result<Self, Self::Error> {
        Err(ErrorKind::Unsupported.into())
    }
}

pub struct OwnedReadHalfUnsend {
    recv_buffer: Cell<Option<Bytes>>,
    receive_future: Cell<Option<Pin<Box<SocketReceiveFuture<TcpDatagram>>>>>,
    socket: ConNetSocket<TcpDatagram>,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    _port_assignment: Option<Rc<TcpPortAssignment>>,
}

pub struct OwnedReadHalf {
    pub inner: CheckSend<OwnedReadHalfUnsend, NodeBound>,
}

impl futures::io::AsyncRead for OwnedReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.inner).poll_read(cx, buf)
    }
}

impl AsyncRead for OwnedReadHalfUnsend {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let received = ready!(self.poll_recv_to_buffer(cx))?;
        Poll::Ready(Ok(self.copy_from_recv_buffer(received, buf, true)))
    }
}

impl agnostic_net::OwnedReadHalf for OwnedReadHalf {
    type Runtime = SimRuntime;

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.local_addr)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.peer_addr)
    }

    fn peek(&mut self, buf: &mut [u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        poll_fn(|cx| self.inner.poll_peek(buf, cx))
    }
}

pub struct OwnedWriteHalfUnsend {
    send_future: Option<Pin<Box<SendFuture>>>,
    dst: Addr,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    _port_assignment: Option<Rc<TcpPortAssignment>>,
    net: SimulatorHandle<ConNet>,
}

pub struct OwnedWriteHalf {
    pub inner: CheckSend<OwnedWriteHalfUnsend, NodeBound>,
}

impl AsyncWrite for OwnedWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }
}

impl AsyncWrite for OwnedWriteHalfUnsend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        ready!(self.as_mut().poll_flush(cx))?;
        let this = &mut *self;
        this.send_future = Some(Box::pin(this.net.with(|net| {
            net.send(WrappedPacket {
                src: Addr {
                    node: NodeId::current(),
                    port: this.dst.port,
                },
                dst: this.dst,
                content: Box::new(TcpDatagram::Data(Bytes::copy_from_slice(buf))),
            })
        })));
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if let Some(x) = &mut self.send_future {
            let result = ready!(x.as_mut().poll(cx));
            self.send_future = None;
            Poll::Ready(result)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        todo!()
    }
}

impl agnostic_net::OwnedWriteHalf for OwnedWriteHalf {
    type Runtime = SimRuntime;

    fn forget(self) {}

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.local_addr)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.peer_addr)
    }
}

impl agnostic_net::TcpStream for TcpStream {
    type Runtime = SimRuntime;

    type OwnedReadHalf = OwnedReadHalf;

    type OwnedWriteHalf = OwnedWriteHalf;

    type ReuniteError = ReuniteError;

    async fn connect<A: agnostic_net::ToSocketAddrs<Self::Runtime>>(
        peer_addr: A,
    ) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        Self::connect_inner(resolve_socket_addrs(peer_addr).await?).await
    }

    fn connect_timeout(
        addr: &SocketAddr,
        timeout: std::time::Duration,
    ) -> impl Future<Output = std::io::Result<Self>> + Send
    where
        Self: Sized,
    {
        SimRuntime::timeout(timeout, Self::connect_inner(*addr))
            .map_ok_or_else(|_| Err(ErrorKind::TimedOut.into()), identity)
    }

    fn peek(&self, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send {
        poll_fn(|cx| self.inner.read.poll_peek(buf, cx))
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.write.local_addr)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.write.peer_addr)
    }

    fn set_ttl(&self, _ttl: u32) -> std::io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn ttl(&self) -> std::io::Result<u32> {
        Err(ErrorKind::Unsupported.into())
    }

    fn set_nodelay(&self, _nodelay: bool) -> std::io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn nodelay(&self) -> std::io::Result<bool> {
        Err(ErrorKind::Unsupported.into())
    }

    fn into_split(self) -> (Self::OwnedReadHalf, Self::OwnedWriteHalf) {
        let this = CheckSend::unwrap_check_send_node(self.inner);
        (
            OwnedReadHalf {
                inner: NodeBound::wrap(this.read),
            },
            OwnedWriteHalf {
                inner: NodeBound::wrap(this.write),
            },
        )
    }

    fn reunite(
        read: Self::OwnedReadHalf,
        write: Self::OwnedWriteHalf,
    ) -> Result<Self, Self::ReuniteError>
    where
        Self: Sized,
    {
        if read.inner.socket.local_port() == write.inner.dst.port {
            Ok(TcpStream {
                inner: NodeBound::wrap(TcpStreamUnSend {
                    write: write.inner.unwrap_check_send_node(),
                    read: read.inner.unwrap_check_send_node(),
                }),
            })
        } else {
            Err(ReuniteError { read, write })
        }
    }
}

impl TcpStream {
    async fn connect_inner(peer_addr: SocketAddr) -> std::io::Result<Self> {
        let id = Id::new();
        let ip = simulator::<IpAddrSimulator>();
        let port_assignment = ip.with(|x| {
            let ip = x.local_ip(peer_addr.ip().is_ipv6());
            x.assign_tcp_ephemeral(ip)
        })?;
        let socket = ConNetSocket::open(id)?;
        let dst_addr = ip.with(|x| x.lookup_socket_addr_udp(peer_addr))?;
        socket
            .send(Addressed {
                addr: dst_addr,
                content: TcpDatagram::Connect {
                    client: port_assignment.addr(),
                    server: peer_addr,
                },
            })
            .await?;
        let received = socket.receive().await?;
        debug_assert!(matches!(received.content, TcpDatagram::Accept));
        let port_assignment = port_assignment.unwrap_check_send_node();
        let local_addr = port_assignment.addr();
        let port_assignment = Some(Rc::new(port_assignment));
        Ok(TcpStream {
            inner: NodeBound::wrap(TcpStreamUnSend {
                write: OwnedWriteHalfUnsend {
                    _port_assignment: port_assignment.clone(),
                    local_addr,
                    send_future: None,
                    dst: dst_addr,
                    peer_addr,
                    net: simulator().unwrap_check_send_sim(),
                },
                read: OwnedReadHalfUnsend {
                    local_addr,
                    recv_buffer: Cell::new(None),
                    _port_assignment: port_assignment,
                    peer_addr,
                    socket: socket.unwrap_check_send_node(),
                    receive_future: Cell::new(None),
                },
            }),
        })
    }
}

impl OwnedReadHalfUnsend {
    fn poll_recv_to_buffer(&self, cx: &mut Context) -> Poll<Result<Bytes>> {
        if let Some(x) = self.recv_buffer.take() {
            return Poll::Ready(Ok(x));
        } else {
            let mut fut = self
                .receive_future
                .take()
                .unwrap_or_else(|| Box::pin(self.socket.receive()));
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(Addressed {
                    content: TcpDatagram::Data(x),
                    ..
                })) => Poll::Ready(Ok(x)),
                Poll::Ready(Ok(_)) => {
                    panic!("received non-data packet after connection established")
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => {
                    self.receive_future.set(Some(fut));
                    Poll::Pending
                }
            }
        }
    }

    fn copy_from_recv_buffer(&self, mut received: Bytes, out: &mut [u8], consume: bool) -> usize {
        let len = received.len().min(out.len());
        out[..len].copy_from_slice(&received[..len]);
        if consume {
            received = received.slice(len..);
        }
        if !received.is_empty() {
            self.recv_buffer.set(Some(received))
        }
        len
    }

    fn poll_peek(&self, buf: &mut [u8], cx: &mut Context) -> Poll<std::io::Result<usize>> {
        let received = ready!(self.poll_recv_to_buffer(cx))?;
        Poll::Ready(Ok(self.copy_from_recv_buffer(received, buf, false)))
    }
}

pub struct TcpListenerUnsend {
    socket: ConNetSocket<TcpDatagram>,
    port_assignment: TcpPortAssignment,
    net: SimulatorHandle<ConNet>,
}

pub struct TcpListener(pub CheckSend<TcpListenerUnsend, NodeBound>);

impl agnostic_net::TcpListener for TcpListener {
    type Runtime = SimRuntime;

    type Stream = TcpStream;

    type Incoming<'a> =
        impl futures::stream::Stream<Item = Result<Self::Stream>> + Send + Sync + Unpin + 'a;

    async fn bind<A: agnostic_net::ToSocketAddrs<Self::Runtime>>(addr: A) -> Result<Self>
    where
        Self: Sized,
    {
        Self::new_inner(resolve_socket_addrs(addr).await?)
    }

    async fn accept(&self) -> Result<(Self::Stream, SocketAddr)> {
        let received = self.0.socket.receive().await?;
        match received.content {
            TcpDatagram::Connect { client, server } => {
                let local_addr = self.0.port_assignment.addr();
                let server_socket_id = Id::new();
                let socket = ConNetSocket::open(server_socket_id)?.unwrap_check_send_node();
                debug_assert_eq!(server, local_addr);
                socket.send(Addressed {
                    addr: received.addr,
                    content: TcpDatagram::Accept,
                });
                Ok((
                    TcpStream {
                        inner: NodeBound::wrap(TcpStreamUnSend {
                            write: OwnedWriteHalfUnsend {
                                local_addr,
                                send_future: None,
                                dst: received.addr,
                                peer_addr: client,
                                _port_assignment: None,
                                net: self.0.net.clone(),
                            },
                            read: OwnedReadHalfUnsend {
                                recv_buffer: Cell::new(None),
                                receive_future: Cell::new(None),
                                socket,
                                local_addr: local_addr,
                                peer_addr: client,
                                _port_assignment: None,
                            },
                        }),
                    },
                    client,
                ))
            }
            TcpDatagram::Data(_) | TcpDatagram::Accept => unreachable!(),
        }
    }

    fn incoming(&self) -> Self::Incoming<'_> {
        Box::pin(futures::stream::unfold(self, |this| async move {
            let con = this.accept().await.map(|x| x.0);
            Some((con, this))
        }))
    }

    fn into_incoming(
        self,
    ) -> impl futures::stream::Stream<Item = std::io::Result<Self::Stream>> + Send {
        futures::stream::unfold(self, |this| async move {
            let con = this.accept().await.map(|x| x.0);
            Some((con, this))
        })
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.0.port_assignment.addr())
    }

    fn set_ttl(&self, _ttl: u32) -> std::io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn ttl(&self) -> std::io::Result<u32> {
        Err(ErrorKind::Unsupported.into())
    }
}

impl TryFrom<std::net::TcpListener> for TcpListener {
    type Error = Error;

    fn try_from(_value: std::net::TcpListener) -> std::result::Result<Self, Self::Error> {
        Err(ErrorKind::Unsupported.into())
    }
}

impl AsFd for TcpListener {
    fn as_fd(&self) -> std::os::unix::prelude::BorrowedFd<'_> {
        unimplemented!()
    }
}

impl AsRawFd for TcpListener {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        unimplemented!()
    }
}

impl TcpListener {
    fn new_inner(addr: SocketAddr) -> Result<Self> {
        let ip = simulator::<IpAddrSimulator>();
        let port_assignment = ip
            .with(|ip| ip.assign_tcp_fixed(addr))?
            .unwrap_check_send_node();
        let socket = ConNetSocket::open(port_assignment.id())?.unwrap_check_send_node();
        Ok(TcpListener(NodeBound::wrap(TcpListenerUnsend {
            socket,
            port_assignment,
            net: simulator().unwrap_check_send_sim(),
        })))
    }
}
