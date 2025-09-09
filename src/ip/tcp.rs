//! # Deviations
//! - When a TcpListener is dropped, the connections created from it remain open and prevent a new listener from being created. Linux closes all connections.
//! - Tcp connections do not time out

use super::{IpAddrSimulator, Result};
use crate::{
    agnostic_lite_runtime::SimRuntime,
    check_send::{CheckSend, Constraint, NodeBound},
    ip::resolve_socket_addrs,
    packet_network::{
        Addr, Addressed, ConNet, ConNetSocket, Packet, SendFuture, SocketReceiveFuture,
        WrappedPacket,
    },
    runtime::{Id, NodeId, spawn},
    simulator::{SimulatorHandle, simulator},
};
use agnostic_net::runtime::RuntimeLite;
use bytes::Bytes;
use either::Either;
use futures::{AsyncRead, FutureExt, TryFutureExt, io::AsyncWrite};
use futures_intrusive::channel::{GenericChannel, LocalChannel};
use rand::random;
use std::{
    cell::Cell,
    collections::{HashMap, hash_map::Entry},
    convert::identity,
    fmt::{Debug, Display},
    future::poll_fn,
    io::{Error, ErrorKind},
    net::{IpAddr, SocketAddr},
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, ready},
};

impl Packet for TcpDatagram {}

struct TcpIncomingConnection {
    client_ip: SocketAddr,
    client_sim: Addr,
}

enum TcpDatagram {
    Connect {
        client: SocketAddr,
        server: SocketAddr,
    },
    Refused,
    Accept,
    Data(Bytes),
}

pub struct TcpStream(pub CheckSend<TcpStreamUnSend, NodeBound>);

pub struct TcpStreamUnSend {
    write: OwnedWriteHalfUnsend,
    read: OwnedReadHalfUnsend,
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut self.0.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.0.write).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.0.write).poll_close(cx)
    }
}

impl futures::io::AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut self.0.read).poll_read(cx, buf)
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
    _port_assignment: Either<Rc<TcpPortAssignment>, Rc<TcpListenHandle>>,
}

pub struct OwnedReadHalf(pub CheckSend<OwnedReadHalfUnsend, NodeBound>);

impl futures::io::AsyncRead for OwnedReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}

impl AsyncRead for OwnedReadHalfUnsend {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let received = ready!(self.poll_recv_to_buffer(cx))?;
        Poll::Ready(Ok(self.copy_from_recv_buffer(received, buf, true)))
    }
}

impl agnostic_net::OwnedReadHalf for OwnedReadHalf {
    type Runtime = SimRuntime;

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.peer_addr)
    }

    fn peek(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send {
        poll_fn(|cx| self.0.poll_peek(buf, cx))
    }
}

pub struct OwnedWriteHalfUnsend {
    send_future: Option<Pin<Box<SendFuture>>>,
    peer_sim: Addr,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    _port_assignment: Either<Rc<TcpPortAssignment>, Rc<TcpListenHandle>>,
    net: SimulatorHandle<ConNet>,
}

pub struct OwnedWriteHalf(pub CheckSend<OwnedWriteHalfUnsend, NodeBound>);

impl AsyncWrite for OwnedWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut *self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut *self.0).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut *self.0).poll_flush(cx)
    }
}

impl AsyncWrite for OwnedWriteHalfUnsend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        ready!(self.as_mut().poll_flush(cx))?;
        let this = &mut *self;
        this.send_future = Some(Box::pin(this.net.with(|net| {
            net.send(WrappedPacket {
                src: Addr {
                    node: NodeId::current(),
                    port: this.peer_sim.port,
                },
                dst: this.peer_sim,
                content: Box::new(TcpDatagram::Data(Bytes::copy_from_slice(buf))),
            })
        })));
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if let Some(x) = &mut self.send_future {
            let result = ready!(x.as_mut().poll(cx));
            self.send_future = None;
            Poll::Ready(result)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        todo!()
    }
}

impl agnostic_net::OwnedWriteHalf for OwnedWriteHalf {
    type Runtime = SimRuntime;

    fn forget(self) {}

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.peer_addr)
    }
}

impl agnostic_net::TcpStream for TcpStream {
    type Runtime = SimRuntime;

    type OwnedReadHalf = OwnedReadHalf;

    type OwnedWriteHalf = OwnedWriteHalf;

    type ReuniteError = ReuniteError;

    async fn connect<A: agnostic_net::ToSocketAddrs<Self::Runtime>>(peer_addr: A) -> Result<Self>
    where
        Self: Sized,
    {
        Self::connect_inner(resolve_socket_addrs(peer_addr).await?).await
    }

    fn connect_timeout(
        addr: &SocketAddr,
        timeout: std::time::Duration,
    ) -> impl Future<Output = Result<Self>> + Send
    where
        Self: Sized,
    {
        SimRuntime::timeout(timeout, Self::connect_inner(*addr))
            .map_ok_or_else(|_| Err(ErrorKind::TimedOut.into()), identity)
    }

    fn peek(&self, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send {
        poll_fn(|cx| self.0.read.poll_peek(buf, cx))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.write.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.write.peer_addr)
    }

    fn set_ttl(&self, _ttl: u32) -> Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn ttl(&self) -> Result<u32> {
        Err(ErrorKind::Unsupported.into())
    }

    fn set_nodelay(&self, _nodelay: bool) -> Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn nodelay(&self) -> Result<bool> {
        Err(ErrorKind::Unsupported.into())
    }

    fn into_split(self) -> (Self::OwnedReadHalf, Self::OwnedWriteHalf) {
        let this = CheckSend::unwrap_check_send_node(self.0);
        (
            OwnedReadHalf(NodeBound::wrap(this.read)),
            OwnedWriteHalf(NodeBound::wrap(this.write)),
        )
    }

    fn reunite(
        read: Self::OwnedReadHalf,
        write: Self::OwnedWriteHalf,
    ) -> Result<Self, Self::ReuniteError>
    where
        Self: Sized,
    {
        if read.0.socket.local_port() == write.0.peer_sim.port {
            Ok(TcpStream(NodeBound::wrap(TcpStreamUnSend {
                write: write.0.unwrap_check_send_node(),
                read: read.0.unwrap_check_send_node(),
            })))
        } else {
            Err(ReuniteError { read, write })
        }
    }
}

impl TcpStream {
    async fn connect_inner(peer_addr: SocketAddr) -> Result<Self> {
        let id = Id::new();
        let ip = simulator::<IpAddrSimulator>();
        let (port_assignment, local_addr) = ip.with(|x| {
            let ip = x.local_ip(peer_addr.ip().is_ipv6());
            x.assign_tcp_ephemeral(ip)
        })?;
        let socket = ConNetSocket::open(id)?;
        let peer_sim = ip.with(|ip| {
            std::io::Result::Ok(Addr {
                node: ip.ip_to_node(peer_addr.ip())?,
                port: ip.tcp_listener_port(),
            })
        })?;
        socket
            .send(Addressed {
                addr: peer_sim,
                content: TcpDatagram::Connect {
                    client: local_addr,
                    server: peer_addr,
                },
            })
            .await?;
        let received = socket.receive().await?;
        debug_assert!(matches!(received.content, TcpDatagram::Accept));
        let port_assignment = Either::Left(Rc::new(port_assignment));
        Ok(TcpStream(NodeBound::wrap(TcpStreamUnSend {
            write: OwnedWriteHalfUnsend {
                _port_assignment: port_assignment.clone(),
                local_addr,
                send_future: None,
                peer_sim,
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
        })))
    }
}

impl OwnedReadHalfUnsend {
    fn poll_recv_to_buffer(&self, cx: &mut Context) -> Poll<Result<Bytes>> {
        if let Some(x) = self.recv_buffer.take() {
            Poll::Ready(Ok(x))
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

    fn poll_peek(&self, buf: &mut [u8], cx: &mut Context) -> Poll<Result<usize>> {
        let received = ready!(self.poll_recv_to_buffer(cx))?;
        Poll::Ready(Ok(self.copy_from_recv_buffer(received, buf, false)))
    }
}

pub struct TcpListenerUnsend {
    listener_handle: Rc<TcpListenHandle>,
    net: SimulatorHandle<ConNet>,
    local_addr: SocketAddr,
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
        let incoming = NodeBound::wrap(self.0.listener_handle.accept()).await;
        let server_socket_id = Id::new();
        let socket = ConNetSocket::open(server_socket_id)?;
        socket
            .send(Addressed {
                addr: incoming.client_sim,
                content: TcpDatagram::Accept,
            })
            .await?;
        let this = &*self.0;
        Ok((
            TcpStream(NodeBound::wrap(TcpStreamUnSend {
                write: OwnedWriteHalfUnsend {
                    local_addr: this.local_addr,
                    send_future: None,
                    peer_sim: incoming.client_sim,
                    peer_addr: incoming.client_ip,
                    _port_assignment: Either::Right(this.listener_handle.clone()),
                    net: self.0.net.clone(),
                },
                read: OwnedReadHalfUnsend {
                    recv_buffer: Cell::new(None),
                    receive_future: Cell::new(None),
                    socket: socket.unwrap_check_send_node(),
                    local_addr: this.local_addr,
                    peer_addr: incoming.client_ip,
                    _port_assignment: Either::Right(this.listener_handle.clone()),
                },
            })),
            incoming.client_ip,
        ))
    }

    fn incoming(&self) -> Self::Incoming<'_> {
        Box::pin(futures::stream::unfold(self, |this| async move {
            let con = this.accept().await.map(|x| x.0);
            Some((con, this))
        }))
    }

    fn into_incoming(self) -> impl futures::stream::Stream<Item = Result<Self::Stream>> + Send {
        futures::stream::unfold(self, |this| async move {
            let con = this.accept().await.map(|x| x.0);
            Some((con, this))
        })
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.local_addr)
    }

    fn set_ttl(&self, _ttl: u32) -> Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn ttl(&self) -> Result<u32> {
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
        ip.with(|ip| {
            let addr = SocketAddr::new(ip.map_bind_addr(addr.ip())?, addr.port());
            let port_assignment = ip.assign_tcp_fixed(addr)?;
            let listener_handle = Rc::new(ip.listen_tcp(port_assignment));
            Ok(TcpListener(NodeBound::wrap(TcpListenerUnsend {
                local_addr: addr,
                listener_handle,
                net: simulator().unwrap_check_send_sim(),
            })))
        })
    }
}

struct TcpPortAssignment {
    id: Id,
}

impl Drop for TcpPortAssignment {
    fn drop(&mut self) {
        simulator::<IpAddrSimulator>().with(|sim| {
            let addr = sim.tcp.to_ip.remove(&self.id).unwrap();
            if let Entry::Occupied(mut x) = sim.tcp.to_id.entry(addr.ip()) {
                x.get_mut().remove(&addr.port());
                if x.get().is_empty() {
                    x.remove();
                }
            } else {
                panic!()
            }
        })
    }
}

struct TcpListenHandle {
    port_assignment: TcpPortAssignment,
    channel: Rc<TcpIncomingChannel>,
}

impl Drop for TcpListenHandle {
    fn drop(&mut self) {
        simulator::<IpAddrSimulator>().with(|sim| {
            let addr = sim.tcp.to_ip[&self.port_assignment.id];
            let removed = sim.tcp.listeners.remove(&addr);
            debug_assert!(Rc::ptr_eq(&self.channel, &removed.unwrap()))
        })
    }
}

impl TcpListenHandle {
    fn accept(&self) -> impl Future<Output = TcpIncomingConnection> {
        self.channel.receive().map(Option::unwrap)
    }
}

impl IpAddrSimulator {
    fn tcp_listener_port(&self) -> Id {
        self.tcp.listener_port
    }

    /// Create an assignement for the requested address, or error if address is in use.
    fn assign_tcp_fixed(&mut self, addr: SocketAddr) -> Result<TcpPortAssignment, std::io::Error> {
        match self
            .tcp
            .to_id
            .entry(addr.ip())
            .or_default()
            .entry(addr.port())
        {
            Entry::Occupied(_) => Err(ErrorKind::AddrInUse.into()),
            Entry::Vacant(x) => {
                let id = Id::new();
                x.insert(id);
                self.tcp.to_ip.insert(id, addr);
                Ok(TcpPortAssignment { id })
            }
        }
    }

    /// Create an assignment for the requested ip address with a random unused port.
    fn assign_tcp_ephemeral(
        &mut self,
        addr: IpAddr,
    ) -> Result<(TcpPortAssignment, SocketAddr), std::io::Error> {
        let ports = self.tcp.to_id.entry(addr).or_default();
        let r: u32 = random();
        const H: usize = 1 << 15;
        let mut port = r as usize % H;
        let step = (r as usize / H) | 1;
        for _ in 0..H {
            let port16 = (port + H) as u16;
            if let Entry::Vacant(x) = ports.entry(port16) {
                let id = Id::new();
                x.insert(id);
                self.tcp.to_ip.insert(id, SocketAddr::new(addr, port16));
                TcpPortAssignment { id };
            } else {
                port = (port + step) % H;
            }
        }
        // example real error: { code: 99, kind: AddrNotAvailable, message: "Cannot assign requested address" }
        Err(std::io::Error::new(
            ErrorKind::AddrNotAvailable,
            "out of tcp ports",
        ))
    }

    fn listen_tcp(&mut self, assignment: TcpPortAssignment) -> TcpListenHandle {
        let addr = self.tcp.to_ip[&assignment.id];
        match self.tcp.listeners.entry(addr) {
            Entry::Occupied(_) => panic!(),
            Entry::Vacant(x) => {
                let channel = Rc::new(GenericChannel::new());
                x.insert(channel.clone());
                TcpListenHandle {
                    port_assignment: assignment,
                    channel,
                }
            }
        }
    }

    pub(super) fn start_tcp_dispatcher(&mut self) {
        let socket = ConNetSocket::<TcpDatagram>::open(self.tcp.listener_port)
            .ok()
            .unwrap();
        spawn(async move {
            let simulator = simulator::<IpAddrSimulator>();
            loop {
                if let Ok(incoming) = socket.receive().await {
                    match incoming.content {
                        TcpDatagram::Connect { client, server } => {
                            let connected = simulator.with(|sim| {
                                if let Some(channel) = sim.tcp.listeners.get(&server) {
                                    channel
                                        .try_send(TcpIncomingConnection {
                                            client_ip: client,
                                            client_sim: incoming.addr,
                                        })
                                        .is_ok()
                                } else {
                                    false
                                }
                            });
                            if !connected {
                                socket
                                    .send(Addressed {
                                        addr: incoming.addr,
                                        content: TcpDatagram::Refused,
                                    })
                                    .await
                                    .ok();
                            }
                        }
                        TcpDatagram::Refused | TcpDatagram::Accept | TcpDatagram::Data(_) => {
                            panic!()
                        }
                    }
                } else {
                    todo!()
                }
            }
        })
        .detach();
    }
}

type TcpIncomingChannel = LocalChannel<TcpIncomingConnection, [TcpIncomingConnection; 16]>;

#[derive(Default)]
pub(super) struct TcpSim {
    to_ip: HashMap<Id, SocketAddr>,
    to_id: HashMap<IpAddr, HashMap<u16, Id>>,
    listeners: HashMap<SocketAddr, Rc<TcpIncomingChannel>>,
    listener_port: Id,
}
