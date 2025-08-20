use crate::{
    ip_addr::IpAddrSimulator,
    packet_network::{Packet, Socket},
    runtime::NodeId,
    simulator::simulator,
};
use bytes::Bytes;
use std::{
    cell::Cell,
    io::{Error, ErrorKind, Result},
    net::{SocketAddr, ToSocketAddrs},
    os::fd::AsRawFd,
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

impl agnostic_net::UdpSocket for UdpSocket {
    async fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        // TODO this may do a real dns lookup
        let addrs = addr.to_socket_addrs()?;
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

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.peer_addr
            .get()
            .ok_or_else(|| Error::new(ErrorKind::NotConnected, "not connected"))
    }

    async fn connect<A: ToSocketAddrs>(&self, addr: A) -> Result<()> {
        let Some(addr) = addr.to_socket_addrs()?.next() else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "could not resolve to any address",
            ));
        };
        self.peer_addr.set(Some(addr));
        Ok(())
    }

    async fn writable(&self) -> Result<()> {
        Ok(())
    }
}
