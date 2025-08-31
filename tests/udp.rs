use agnostic_net::UdpSocket as _;
use deterministic_simulation_core::{
    ip_addr::IpAddrSimulator,
    packet_network::{ConNet, perfect_connectivity},
    runtime::{NodeId, Runtime},
    simulator::{add_simulator, simulator},
    time::sleep,
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

async fn receiver(from: Option<NodeId>) -> Instant {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 23)).await.unwrap();
    let mut buf = [0; 8];
    let recv = socket.recv_from(&mut buf);
    if let Some(from) = from {
        assert!(recv.await.unwrap() == (4, SocketAddr::new(get_ip(from), 22)));
        assert_eq!(&buf, b"ping\0\0\0\0");
        Instant::now()
    } else {
        let x = recv.await;
        panic!("recv completed: {x:?}")
    }
}

async fn sender(dst: NodeId) -> Instant {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 22)).await.unwrap();
    socket.local_addr().unwrap();
    let peer = socket.peer_addr();
    assert!(peer.is_err_and(|e| e.kind() == ErrorKind::NotConnected));
    let send = socket.send_to(b"ping", SocketAddr::new(get_ip(dst), 23));
    assert_eq!(send.await.unwrap(), 4);
    Instant::now()
}

const LATENCY: Duration = Duration::from_millis(100);

fn setup_net(node_count: usize) -> Vec<NodeId> {
    add_simulator(ConNet::new(perfect_connectivity(LATENCY)));
    add_simulator(IpAddrSimulator::default());
    (0..node_count).map(|_| NodeId::create_node()).collect()
}

#[test]
fn send() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn(sender(n2));
        let t2 = n2.spawn(receiver(Some(n1)));
        assert!(t1.await.unwrap() + LATENCY == t2.await.unwrap());
    });
}

#[test]
fn send_early() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn(sender(n2)).await;
        let t2 = n2.spawn(receiver(Some(n1)));
        assert!(t1.unwrap() + LATENCY == t2.await.unwrap());
    });
}

#[test]
fn send_too_early() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn(sender(n2)).await;
        sleep(LATENCY + LATENCY).await;
        let t2 = n2.spawn(receiver(None));
        assert!(t1.unwrap() + LATENCY == t2.await.unwrap());
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
