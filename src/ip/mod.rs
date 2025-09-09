pub mod tcp;
pub mod udp;

use crate::{agnostic_lite_runtime::SimRuntime, runtime::NodeId, simulator::Simulator};
use agnostic_net::ToSocketAddrs;
use std::{
    collections::HashMap,
    io::{Error, ErrorKind},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tcp::TcpSim;
use udp::UdpSim;

type Result<T, E = std::io::Error> = std::result::Result<T, E>;

/// Assign Ip-addresses to nodes.
///
/// Each node except for the initial node has exactly one ip-v4 and one ip-v6 address.
/// The initial node has no address.
pub struct IpAddrSimulator {
    to_node: HashMap<IpAddr, NodeId>,
    to_ip: HashMap<NodeId, (Ipv4Addr, Ipv6Addr)>,
    gen_v4: Box<dyn FnMut() -> Ipv4Addr>,
    gen_v6: Box<dyn FnMut() -> Ipv6Addr>,
    override_next: Option<(Ipv4Addr, Ipv6Addr)>,
    udp: UdpSim,
    tcp: TcpSim,
}

pub struct LookupError {
    addr: IpAddr,
}

impl From<LookupError> for std::io::Error {
    fn from(value: LookupError) -> Self {
        std::io::Error::new(
            std::io::ErrorKind::NetworkUnreachable,
            format!("no node with this ip address: {:?}", value.addr),
        )
    }
}

impl Default for IpAddrSimulator {
    fn default() -> Self {
        Self::new(Self::private_ipv4(), Self::unique_local_address_hosts())
    }
}

impl IpAddrSimulator {
    pub fn ip_to_node(&self, addr: IpAddr) -> Result<NodeId, LookupError> {
        self.to_node.get(&addr).copied().ok_or(LookupError { addr })
    }

    pub fn node_to_ip(&self, node: NodeId, v6: bool) -> IpAddr {
        if node == NodeId::INIT {
            panic!("init node is not assigned an ip address");
        }
        let ips = self.to_ip.get(&node).unwrap();
        if v6 {
            IpAddr::V6(ips.1)
        } else {
            IpAddr::V4(ips.0)
        }
    }

    pub fn local_ip(&self, v6: bool) -> IpAddr {
        self.node_to_ip(NodeId::current(), v6)
    }

    /// Override the address given to the next node created.
    ///
    /// This should be immediately followed by a call to [NodeId::create_new].
    /// The callbacks passed to the simulator at construction will not be invoked.
    pub fn override_next_node_addr(&mut self, v4: Ipv4Addr, v6: Ipv6Addr) {
        assert!(
            self.override_next.replace((v4, v6)).is_none(),
            "override already set"
        );
    }

    /// Create a simulator.
    ///
    /// the provided callbacks are used to generate addresses for new nodes.
    pub fn new(gen_v4: Box<dyn FnMut() -> Ipv4Addr>, gen_v6: Box<dyn FnMut() -> Ipv6Addr>) -> Self {
        IpAddrSimulator {
            to_node: HashMap::default(),
            to_ip: HashMap::default(),
            gen_v4,
            gen_v6,
            override_next: None,
            udp: Default::default(),
            tcp: Default::default(),
        }
    }

    pub fn private_ipv4() -> Box<dyn FnMut() -> Ipv4Addr> {
        // avoid subnet and broadcast address.
        const BASE: u32 = 10 << 24;
        const LIMIT: u32 = (11 << 24) - 1;
        let mut counter = BASE;
        Box::new(move || {
            counter += 1;
            assert!(counter < LIMIT, "out of ipv4 addresses");
            Ipv4Addr::from_bits(counter)
        })
    }

    pub fn unique_local_address_hosts() -> Box<dyn FnMut() -> Ipv6Addr> {
        // generate from 2^8 contiguous "random" ids to match size of private ipv4 space.
        const BASE: u64 = 0xfd_e0e09f64 << 24;
        const LIMIT: u64 = BASE + (1 << 24);
        let mut counter = BASE;
        Box::new(move || {
            counter += 1;
            assert!(counter < LIMIT, "out of ipv6 addresses");
            Ipv6Addr::from_bits((counter as u128) << 64)
        })
    }

    fn map_bind_addr(&self, a: IpAddr) -> Result<IpAddr> {
        let local = self.local_ip(a.is_ipv6());
        if a.is_unspecified() {
            Ok(local)
        } else if a.is_loopback() || a.is_multicast() {
            Err(Error::new(
                ErrorKind::InvalidData,
                "unsupported special ip address",
            ))
        } else if a == local {
            Ok(a)
        } else {
            Err(ErrorKind::AddrNotAvailable.into())
        }
    }
}

impl Simulator for IpAddrSimulator {
    fn create_node(&mut self) {
        let ips = self
            .override_next
            .take()
            .unwrap_or_else(|| ((self.gen_v4)(), (self.gen_v6)()));
        let node = NodeId::current();
        let mut insert = |ip| {
            if let Some(old_node) = self.to_node.insert(ip, node) {
                panic!("ip {ip} assigned to multiple nodes: {old_node:?} and {node:?}");
            }
        };
        insert(IpAddr::V4(ips.0));
        insert(IpAddr::V6(ips.1));
        assert!(self.to_ip.insert(node, ips).is_none());
        self.start_node();
    }

    fn start_node(&mut self) {
        self.start_tcp_dispatcher();
    }
}

async fn resolve_socket_addrs(addr: impl ToSocketAddrs<SimRuntime>) -> std::io::Result<SocketAddr> {
    addr.to_socket_addrs().await?.next().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "could not resolve to any address")
    })
}
