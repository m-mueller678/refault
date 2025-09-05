use crate::{
    agnostic_lite_runtime::SimRuntime,
    check_send::{CheckSend, Constraint, NodeBound},
    ip_addr::IpAddrSimulator,
    packet_network::{Addressed, ConNetSocket, Packet, SocketReceiveFuture},
    runtime::NodeId,
    simulator::{SimulatorHandle, simulator},
};
use agnostic_net::ToSocketAddrs;
use bytes::Bytes;
use either::Either::{Left, Right};
use futures::FutureExt;
use std::{
    cell::Cell,
    future::{poll_fn, ready},
    io::{Error, ErrorKind, Result},
    net::SocketAddr,
    os::fd::{AsFd, AsRawFd},
    pin::Pin,
    task::ready,
    task::{Context, Poll},
};

pub struct UdpSocket(pub CheckSend<UdpSocketUnSend, NodeBound>);

pub struct UdpSocketUnSend {
    peer_addr: Cell<Option<SocketAddr>>,
    receive_state: Cell<ReceiveState>,
    socket: ConNetSocket<UdpDatagram>,
    local_addr: SocketAddr,
    ip: SimulatorHandle<IpAddrSimulator>,
}

enum ReceiveState {
    None,
    Taken,
    Future(Pin<Box<SocketReceiveFuture<UdpDatagram>>>),
    Peeked(UdpDatagram),
}

struct UdpDatagram {
    bytes: Bytes,
    src: SocketAddr,
    dst: SocketAddr,
}

impl Packet for UdpDatagram {}

impl AsRawFd for UdpSocket {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        unimplemented!()
    }
}

impl AsFd for UdpSocket {
    fn as_fd(&self) -> std::os::unix::prelude::BorrowedFd<'_> {
        unimplemented!()
    }
}

impl TryFrom<std::net::UdpSocket> for UdpSocket {
    type Error = Error;

    fn try_from(_value: std::net::UdpSocket) -> std::result::Result<Self, Self::Error> {
        Err(unsupported())
    }
}

impl agnostic_net::UdpSocket for UdpSocket {
    type Runtime = SimRuntime;

    async fn bind<A: ToSocketAddrs<Self::Runtime>>(addr: A) -> Result<Self> {
        // TODO this may do a real dns lookup
        let addrs = addr.to_socket_addrs().await?;
        let mut last_err = None;
        for addr in addrs {
            match UdpSocket::bind_addr(addr) {
                Ok(socket) => return Ok(socket),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            Error::new(ErrorKind::InvalidInput, "could not resolve to any address")
        }))
    }

    async fn connect<A: ToSocketAddrs<Self::Runtime>>(&self, addr: A) -> Result<()> {
        let Some(addr) = addr.to_socket_addrs().await?.next() else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "could not resolve to any address",
            ));
        };
        self.0.peer_addr.set(Some(addr));
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.0.peer()
    }

    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let (recv, peer_addr) = {
                let inner = &*self.0;
                (self.recv_inner(&self.0), inner.peer()?)
            };
            let packet = recv.await?;
            debug_assert!(packet.dst == self.0.local_addr);
            if peer_addr != packet.src {
                continue;
            }
            buf[..packet.bytes.len()].copy_from_slice(&packet.bytes);
            break Ok(packet.bytes.len());
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        let packet: UdpDatagram = self.recv_inner(&self.0).await?;
        debug_assert!(packet.dst == self.0.local_addr);
        buf[..packet.bytes.len()].copy_from_slice(&packet.bytes);
        Ok((packet.bytes.len(), packet.src))
    }

    async fn send(&self, buf: &[u8]) -> std::io::Result<usize> {
        let dst = self
            .0
            .peer_addr
            .get()
            .ok_or_else(|| Error::new(ErrorKind::NotConnected, "not conncted"))?;
        let send_inner = self.send_inner(buf, dst);
        send_inner?.await
    }

    async fn send_to<A: agnostic_net::ToSocketAddrs<Self::Runtime>>(
        &self,
        buf: &[u8],
        target: A,
    ) -> std::io::Result<usize> {
        let dst = resolve_addr(target).await?;
        self.send_inner(buf, dst)?.await
    }

    async fn peek(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let peer = self.0.peer()?;
        loop {
            return match poll_fn(|cx| self.poll_recv(cx)).await {
                Ok(x) => {
                    if x.src != peer {
                        continue;
                    }
                    buf[..x.bytes.len()].copy_from_slice(&x.bytes);
                    let len = x.bytes.len();
                    self.put_receive_state(ReceiveState::Peeked(x));
                    Ok(len)
                }
                Err(e) => {
                    self.put_receive_state(ReceiveState::None);
                    Err(e)
                }
            };
        }
    }

    async fn peek_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        match poll_fn(|cx| self.poll_recv(cx)).await {
            Ok(x) => {
                buf[..x.bytes.len()].copy_from_slice(&x.bytes);
                let ret = (x.bytes.len(), x.src);
                self.put_receive_state(ReceiveState::Peeked(x));
                Ok(ret)
            }
            Err(e) => {
                self.put_receive_state(ReceiveState::None);
                Err(e)
            }
        }
    }

    fn join_multicast_v4(
        &self,
        _multiaddr: std::net::Ipv4Addr,
        _interface: std::net::Ipv4Addr,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn join_multicast_v6(
        &self,
        _multiaddr: &std::net::Ipv6Addr,
        _interface: u32,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn leave_multicast_v4(
        &self,
        _multiaddr: std::net::Ipv4Addr,
        _interface: std::net::Ipv4Addr,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn leave_multicast_v6(
        &self,
        _multiaddr: &std::net::Ipv6Addr,
        _interface: u32,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn multicast_loop_v4(&self) -> std::io::Result<bool> {
        Err(unsupported())
    }

    fn set_multicast_loop_v4(&self, _on: bool) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn multicast_ttl_v4(&self) -> std::io::Result<u32> {
        Err(unsupported())
    }

    fn set_multicast_ttl_v4(&self, _ttl: u32) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn multicast_loop_v6(&self) -> std::io::Result<bool> {
        Err(unsupported())
    }

    fn set_multicast_loop_v6(&self, _on: bool) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn set_ttl(&self, _ttl: u32) -> std::io::Result<()> {
        Err(unsupported())
    }

    fn ttl(&self) -> std::io::Result<u32> {
        Ok(0)
    }

    fn set_broadcast(&self, broadcast: bool) -> std::io::Result<()> {
        if broadcast {
            Err(unsupported())
        } else {
            Ok(())
        }
    }

    fn broadcast(&self) -> std::io::Result<bool> {
        Ok(false)
    }

    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<(usize, SocketAddr)>> {
        let packet = ready!(self.poll_recv(cx));
        self.put_receive_state(ReceiveState::None);
        let packet = packet?;
        buf[..packet.bytes.len()].copy_from_slice(&packet.bytes);
        Poll::Ready(Ok((packet.bytes.len(), packet.src)))
    }

    fn poll_send_to(
        &self,
        _cx: &mut std::task::Context<'_>,
        _buf: &[u8],
        _target: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        Poll::Ready(Err(unsupported()))
    }
}

impl UdpSocket {
    fn bind_addr(local_addr: SocketAddr) -> Result<Self> {
        let mut ip = local_addr.ip();
        let port = if ip.is_loopback() || ip.is_multicast() {
            unimplemented!();
        } else {
            let addr = simulator::<IpAddrSimulator>().with(|x| {
                if ip.is_unspecified() {
                    ip = x.local_ip(ip.is_ipv6());
                }
                x.lookup_socket_addr(SocketAddr::new(ip, local_addr.port()))
            })?;
            if addr.node != NodeId::current() {
                return Err(Error::new(
                    ErrorKind::AddrNotAvailable,
                    "specified ip address does not belong to this node",
                ));
            }
            addr.port
        };
        Ok(UdpSocket(NodeBound::wrap(UdpSocketUnSend {
            socket: ConNetSocket::open(port)?.unwrap_check_send_node(),
            local_addr: SocketAddr::new(ip, local_addr.port()),
            ip: simulator().unwrap_check_send_sim(),
            peer_addr: Cell::new(None),
            receive_state: Cell::new(ReceiveState::None),
        })))
    }
}

impl UdpSocket {
    fn send_inner(
        &self,
        bytes: &[u8],
        dst: SocketAddr,
    ) -> Result<impl Send + Future<Output = Result<usize>>> {
        let future = {
            assert!(dst.is_ipv4() == self.0.local_addr.is_ipv4());
            let packet = UdpDatagram {
                bytes: Bytes::copy_from_slice(bytes),
                src: self.0.local_addr,
                dst,
            };
            let addr = self.0.ip.with(|ip| ip.lookup_socket_addr(packet.dst))?;
            self.0.socket.send(Addressed {
                addr,
                content: packet,
            })
        };
        let len = bytes.len();
        Ok(future.map(move |x| x.map(|()| len)))
    }

    fn map_content(x: Result<Addressed<UdpDatagram>>) -> Result<UdpDatagram> {
        x.map(|x| x.content)
    }

    fn recv_inner(
        &self,
        inner: &UdpSocketUnSend,
    ) -> impl Send + Future<Output = Result<UdpDatagram>> + use<> {
        match inner.receive_state.replace(ReceiveState::None) {
            ReceiveState::None => Left(self.0.socket.receive().map(Self::map_content)),
            ReceiveState::Taken => unreachable!(),
            ReceiveState::Peeked(p) => Right(Left(ready(Ok(p)))),
            ReceiveState::Future(x) => Right(Right(x.map(Self::map_content))),
        }
    }

    fn poll_recv(&self, cx: &mut Context) -> Poll<Result<UdpDatagram>> {
        let inner = &*self.0;
        let mut fut = match inner.receive_state.replace(ReceiveState::Taken) {
            ReceiveState::None => Box::pin(self.0.socket.receive()),
            ReceiveState::Future(f) => f,
            ReceiveState::Taken => unreachable!(),
            ReceiveState::Peeked(x) => {
                return Poll::Ready(Ok(x));
            }
        };
        match fut.as_mut().poll(cx) {
            Poll::Ready(x) => Poll::Ready(x.map(|x| x.content)),
            std::task::Poll::Pending => {
                inner.receive_state.set(ReceiveState::Future(fut));
                Poll::Pending
            }
        }
    }

    fn put_receive_state(&self, s: ReceiveState) {
        assert!(matches!(
            self.0.receive_state.replace(s),
            ReceiveState::Taken
        ))
    }
}
impl UdpSocketUnSend {
    fn peer(&self) -> Result<SocketAddr> {
        self.peer_addr
            .get()
            .ok_or_else(|| ErrorKind::NotConnected.into())
    }
}

async fn resolve_addr(addr: impl ToSocketAddrs<SimRuntime>) -> Result<SocketAddr> {
    addr.to_socket_addrs()
        .await?
        .next()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "could not resolve to any address"))
}

fn unsupported() -> Error {
    ErrorKind::Unsupported.into()
}
