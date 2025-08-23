use crate::{
    packet_network::Addr,
    runtime::{IdRange, NodeId},
    simulator::Simulator,
};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

/// Assign Ip-addresses to nodes.
///
/// Each node except for the initial node has exactly one ip-v4 and one ip-v6 address.
/// The initial node has no address.
pub struct IpAddrSimulator {
    v4_ports: IdRange,
    v6_ports: IdRange,
    to_node: HashMap<IpAddr, NodeId>,
    to_ip: HashMap<NodeId, (Ipv4Addr, Ipv6Addr)>,
    gen_v4: Box<dyn FnMut() -> Ipv4Addr>,
    gen_v6: Box<dyn FnMut() -> Ipv6Addr>,
    override_next: Option<(Ipv4Addr, Ipv6Addr)>,
}

pub struct LookupError {
    addr: SocketAddr,
}

impl From<LookupError> for std::io::Error {
    fn from(value: LookupError) -> Self {
        std::io::Error::new(
            std::io::ErrorKind::NetworkUnreachable,
            format!("no node with this ip address: {:?}", value.addr),
        )
    }
}

impl IpAddrSimulator {
    // Translate an ip based socket address into a simulated network address.
    pub fn lookup_socket_addr(&self, addr: SocketAddr) -> Result<Addr, LookupError> {
        let node = *self.to_node.get(&addr.ip()).ok_or(LookupError { addr })?;
        let port = match addr.ip() {
            IpAddr::V4(_) => self.v4_ports.get(addr.port() as usize),
            IpAddr::V6(_) => self.v6_ports.get(addr.port() as usize),
        };
        Ok(Addr { node, port })
    }

    pub fn local_ip(&self, v6: bool) -> IpAddr {
        let ips = self.to_ip.get(&NodeId::current()).unwrap();
        if v6 {
            IpAddr::V6(ips.1)
        } else {
            IpAddr::V4(ips.0)
        }
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
            v4_ports: IdRange::new(1 << 16),
            v6_ports: IdRange::new(1 << 16),
            to_node: HashMap::default(),
            to_ip: HashMap::default(),
            gen_v4,
            gen_v6,
            override_next: None,
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
    }
}
