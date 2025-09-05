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
    task::{Context, Poll, ready},
};

use crate::{
    agnostic_lite_runtime::SimRuntime,
    check_send::{CheckSend, Constraint, NodeBound},
    ip_addr::IpAddrSimulator,
    packet_network::{
        Addr, Addressed, ConNet, ConNetSocket, Packet, SendFuture, SocketReceiveFuture,
        WrappedPacket,
    },
    runtime::{Id, NodeId},
    simulator::{SimulatorHandle, simulator},
};

impl Packet for TcpDatagram {}

enum TcpDatagram {
    Connect {
        id: Id,
        src: SocketAddr,
        dst: SocketAddr,
    },
    Accept,
    Data(Bytes),
}

pub struct TcpStream {
    pub inner: CheckSend<TcpSocketUnSend, NodeBound>,
}

pub struct TcpSocketUnSend {
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
    local_ip: SocketAddr,
    peer_ip: SocketAddr,
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
        Ok(self.inner.local_ip)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.peer_ip)
    }

    fn peek(&mut self, buf: &mut [u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        poll_fn(|cx| self.inner.poll_peek(buf, cx))
    }
}

pub struct OwnedWriteHalfUnsend {
    send_future: Option<Pin<Box<SendFuture>>>,
    dst: Addr,
    ip_src: SocketAddr,
    ip_dst: SocketAddr,
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
        Ok(self.inner.ip_src)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.ip_dst)
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
        let Some(peer_addr) = peer_addr.to_socket_addrs().await?.next() else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "could not resolve to any address",
            ));
        };
        Self::connect_inner(peer_addr).await
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

    fn peek(&self, buf: &mut [u8]) -> impl Future<Output = Result<usize, Error>> + Send {
        poll_fn(|cx| self.inner.read.poll_peek(buf, cx))
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.write.ip_src)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.inner.write.ip_dst)
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
                inner: NodeBound::wrap(TcpSocketUnSend {
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
        let local_ip = ip.with(|x| x.local_ip(peer_addr.ip().is_ipv6()));
        // TODO select port to avoid collisions
        // TODO avoid id collision on local connection
        let port = 20_000;
        let local_addr = SocketAddr::new(local_ip, port);
        let socket = ConNetSocket::open(id)?;
        let dst_addr = ip.with(|x| x.lookup_socket_addr(peer_addr))?;
        socket
            .send(Addressed {
                addr: dst_addr,
                content: TcpDatagram::Connect {
                    id,
                    src: local_addr,
                    dst: peer_addr,
                },
            })
            .await?;
        let received = socket.receive().await?;
        debug_assert!(matches!(received.content, TcpDatagram::Accept));
        Ok(TcpStream {
            inner: NodeBound::wrap(TcpSocketUnSend {
                write: OwnedWriteHalfUnsend {
                    send_future: None,
                    dst: dst_addr,
                    ip_src: local_addr,
                    ip_dst: peer_addr,
                    net: simulator().unwrap_check_send_sim(),
                },
                read: OwnedReadHalfUnsend {
                    recv_buffer: Cell::new(None),
                    local_ip: local_addr,
                    peer_ip: peer_addr,
                    socket: socket.unwrap_check_send_node(),
                    receive_future: Cell::new(None),
                },
            }),
        })
    }
}

impl OwnedReadHalfUnsend {
    fn poll_recv_to_buffer(&self, cx: &mut Context) -> Poll<Result<Bytes, Error>> {
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
