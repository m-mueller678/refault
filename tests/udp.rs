use agnostic_net::UdpSocket as _;
use deterministic_simulation_core::{
    ip_addr::IpAddrSimulator,
    packet_network::{ConNet, perfect_connectivity},
    runtime::{NodeId, Runtime},
    simulator::{add_simulator, simulator},
    udp::UdpSocket,
};
use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

fn get_ip(n: NodeId) -> IpAddr {
    simulator::<IpAddrSimulator>().with(|s| s.get_ip_for_node(n, false))
}

#[test]
fn udp_send() {
    Runtime::new().run(|| async {
        let latency = Duration::from_millis(100);
        add_simulator(ConNet::new(perfect_connectivity(latency)));
        add_simulator(IpAddrSimulator::default());
        let n1 = NodeId::create_node();
        let n2 = NodeId::create_node();
        let t1 = n1.spawn(async move {
            let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 22)).await.unwrap();
            socket.local_addr().unwrap();
            let peer = socket.peer_addr();
            assert!(peer.is_err_and(|e| e.kind() == ErrorKind::NotConnected));
            let send = socket.send_to(b"ping", SocketAddr::new(get_ip(n2), 23));
            assert_eq!(send.await.unwrap(), 4);
            Instant::now()
        });
        let t2 = n2.spawn(async move {
            let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 23)).await.unwrap();
            let mut buf = [0; 8];
            let recv = socket.recv_from(&mut buf);
            assert!(recv.await.unwrap() == (4, SocketAddr::new(get_ip(n1), 22)));
            assert_eq!(&buf, b"ping\0\0\0\0");
            Instant::now()
        });
        assert!(t1.await.unwrap() + latency == t2.await.unwrap());
    });
}

#[test]
fn gen_ipv4() {
    let mut generator = IpAddrSimulator::private_ipv4();
    assert_eq!(generator(), Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(generator(), Ipv4Addr::new(10, 0, 0, 2));
}

#[test]
fn gen_ipv6() {
    let mut generator = IpAddrSimulator::unique_local_address_hosts();
    let a = generator();
    let b = generator();
    assert!(a.is_unique_local());
    assert!(b.to_bits() - a.to_bits() == 1u128 << 64);
}
