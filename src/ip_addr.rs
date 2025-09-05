use rand::random;

use crate::{
    check_send::{CheckSend, Constraint, NodeBound},
    packet_network::Addr,
    runtime::{Id, IdRange, NodeId},
    simulator::{Simulator, simulator},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    io::ErrorKind,
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
    tcp_to_ip: HashMap<Id, SocketAddr>,
    tcp_to_id: HashMap<IpAddr, HashMap<u16, Id>>,
}

pub struct TcpPortAssignment {
    id: Id,
}

impl Drop for TcpPortAssignment {
    fn drop(&mut self) {
        simulator::<IpAddrSimulator>().with(|sim| {
            let addr = sim.tcp_to_ip[&self.id];
            if let Entry::Occupied(mut x) = sim.tcp_to_id.entry(addr.ip()) {
                x.get_mut().remove(&addr.port());
                if x.get().is_empty() {
                    x.remove();
                }
            } else {
                unreachable!()
            }
        })
    }
}

impl TcpPortAssignment {
    pub fn id(&self) -> Id {
        self.id
    }

    pub fn addr(&self) -> SocketAddr {
        simulator::<IpAddrSimulator>().with(|x| x.tcp_to_ip[&self.id])
    }
}

impl TcpPortAssignment {}

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

impl Default for IpAddrSimulator {
    fn default() -> Self {
        Self::new(Self::private_ipv4(), Self::unique_local_address_hosts())
    }
}

impl IpAddrSimulator {
    /// Create an assignement for the requested address, or error if address is in use.
    pub fn assign_tcp_fixed(
        &mut self,
        addr: SocketAddr,
    ) -> Result<CheckSend<TcpPortAssignment, NodeBound>, std::io::Error> {
        match self
            .tcp_to_id
            .entry(addr.ip())
            .or_default()
            .entry(addr.port())
        {
            Entry::Occupied(_) => return Err(ErrorKind::AddrInUse.into()),
            Entry::Vacant(x) => {
                let id = Id::new();
                x.insert(id);
                self.tcp_to_ip.insert(id, addr);
                Ok(NodeBound::wrap(TcpPortAssignment { id }))
            }
        }
    }

    /// Create an assignment for the requested ip address with a random unused port.
    pub fn assign_tcp_ephemeral(
        &mut self,
        addr: IpAddr,
    ) -> Result<CheckSend<TcpPortAssignment, NodeBound>, std::io::Error> {
        let ports = self.tcp_to_id.entry(addr).or_default();
        let r: u32 = random();
        const H: usize = 1 << 15;
        let mut port = r as usize % H;
        let step = (r as usize / H) | 1;
        for _ in 0..H {
            let port16 = (port + H) as u16;
            if let Entry::Vacant(x) = ports.entry(port16) {
                let id = Id::new();
                x.insert(id);
                self.tcp_to_ip.insert(id, SocketAddr::new(addr, port16));
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
    // Translate an ip based socket address into a simulated network address.
    pub fn lookup_socket_addr(&self, addr: SocketAddr) -> Result<Addr, LookupError> {
        let node = *self.to_node.get(&addr.ip()).ok_or(LookupError { addr })?;
        let port = match addr.ip() {
            IpAddr::V4(_) => self.v4_ports.get(addr.port() as usize),
            IpAddr::V6(_) => self.v6_ports.get(addr.port() as usize),
        };
        Ok(Addr { node, port })
    }

    pub fn get_ip_for_node(&self, node: NodeId, v6: bool) -> IpAddr {
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
        self.get_ip_for_node(NodeId::current(), v6)
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
            tcp_to_ip: HashMap::new(),
            tcp_to_id: HashMap::new(),
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
