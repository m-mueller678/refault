use crate::{
    agnostic_lite_runtime::SimRuntime,
    ip_addr::IpAddrSimulator,
    packet_network::{Packet, Socket},
    runtime::NodeId,
    simulator::simulator,
};
use agnostic_net::ToSocketAddrs;
use bytes::Bytes;
use futures::StreamExt;
use std::{
    cell::Cell,
    io::{Error, ErrorKind, Result},
    net::SocketAddr,
    os::fd::{AsFd, AsRawFd},
};

pub struct UdpSocket {
    socket: Socket<UdpDatagram>,
    local_addr: SocketAddr,
    peer_addr: Cell<Option<SocketAddr>>,
}

struct UdpDatagram {
    bytes: Bytes,
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

    fn try_from(value: std::net::UdpSocket) -> std::result::Result<Self, Self::Error> {
        Err(Error::new(ErrorKind::Unsupported, "unsupported"))
    }
}

//TODO fix
unsafe impl Send for UdpSocket {}
unsafe impl Sync for UdpSocket {}

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
        self.peer_addr.set(Some(addr));
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.peer_addr
            .get()
            .ok_or_else(|| Error::new(ErrorKind::NotConnected, "not connected"))
    }

    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let packet = self.socket.next().await;
        Ok(todo!())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        todo!()
    }

    async fn send(&self, buf: &[u8]) -> std::io::Result<usize> {
        todo!()
    }

    async fn send_to<A: agnostic_net::ToSocketAddrs<Self::Runtime>>(
        &self,
        buf: &[u8],
        target: A,
    ) -> std::io::Result<usize> {
        todo!()
    }

    async fn peek(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        todo!()
    }

    async fn peek_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        todo!()
    }

    fn join_multicast_v4(
        &self,
        multiaddr: std::net::Ipv4Addr,
        interface: std::net::Ipv4Addr,
    ) -> std::io::Result<()> {
        todo!()
    }

    fn join_multicast_v6(
        &self,
        multiaddr: &std::net::Ipv6Addr,
        interface: u32,
    ) -> std::io::Result<()> {
        todo!()
    }

    fn leave_multicast_v4(
        &self,
        multiaddr: std::net::Ipv4Addr,
        interface: std::net::Ipv4Addr,
    ) -> std::io::Result<()> {
        todo!()
    }

    fn leave_multicast_v6(
        &self,
        multiaddr: &std::net::Ipv6Addr,
        interface: u32,
    ) -> std::io::Result<()> {
        todo!()
    }

    fn multicast_loop_v4(&self) -> std::io::Result<bool> {
        todo!()
    }

    fn set_multicast_loop_v4(&self, on: bool) -> std::io::Result<()> {
        todo!()
    }

    fn multicast_ttl_v4(&self) -> std::io::Result<u32> {
        todo!()
    }

    fn set_multicast_ttl_v4(&self, ttl: u32) -> std::io::Result<()> {
        todo!()
    }

    fn multicast_loop_v6(&self) -> std::io::Result<bool> {
        todo!()
    }

    fn set_multicast_loop_v6(&self, on: bool) -> std::io::Result<()> {
        todo!()
    }

    fn set_ttl(&self, ttl: u32) -> std::io::Result<()> {
        todo!()
    }

    fn ttl(&self) -> std::io::Result<u32> {
        todo!()
    }

    fn set_broadcast(&self, broadcast: bool) -> std::io::Result<()> {
        todo!()
    }

    fn broadcast(&self) -> std::io::Result<bool> {
        todo!()
    }

    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<(usize, SocketAddr)>> {
        todo!()
    }

    fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        todo!()
    }
}

impl UdpSocket {
    fn bind_addr(local_addr: SocketAddr) -> Result<Self> {
        let mut ip = local_addr.ip();
        let port = if ip.is_loopback() || ip.is_multicast() {
            unimplemented!();
        } else {
            simulator::<IpAddrSimulator>()
                .with(|x| {
                    if ip.is_unspecified() {
                        ip = x.local_ip(ip.is_ipv6());
                    }
                    x.lookup_socket_addr(SocketAddr::new(ip, local_addr.port()))
                })
                .filter(|x| x.node == NodeId::current())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::AddrNotAvailable,
                        "specified ip address does not belong to this node",
                    )
                })?
                .port
        };
        let socket = Socket::new(port)?;
        Ok(UdpSocket {
            socket,
            local_addr: SocketAddr::new(ip, local_addr.port()),
            peer_addr: Cell::new(None),
        })
    }
}
