use std::net::Ipv4Addr;

use agnostic_net::UdpSocket as _;
use deterministic_simulation_core::{
    ip_addr::IpAddrSimulator,
    runtime::{NodeId, Runtime},
    simulator::add_simulator,
    udp::UdpSocket,
};

#[test]
fn ping_pong() {
    Runtime::new().run(|| async {
        add_simulator(IpAddrSimulator::default());
        NodeId::create_node()
            .spawn(async move {
                let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 22)).await;
            })
            .await
            .unwrap();
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
