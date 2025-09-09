use agnostic_lite::RuntimeLite;
use agnostic_net::{TcpListener as _, TcpStream as _, UdpSocket as _};
use futures::{AsyncReadExt, AsyncWriteExt};
use refault::{
    agnostic_lite_runtime::SimRuntime,
    ip::{
        IpAddrSimulator,
        tcp::{TcpListener, TcpStream},
        udp::UdpSocket,
    },
    packet_network::{ConNet, perfect_connectivity},
    runtime::{NodeId, Runtime, spawn},
    simulator::{add_simulator, simulator},
    time::sleep,
};
use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

fn get_ip(n: NodeId) -> IpAddr {
    simulator::<IpAddrSimulator>().with(|s| s.node_to_ip(n, false))
}

async fn udp_receiver(from: Option<NodeId>) -> Instant {
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

async fn udp_sender(dst: NodeId) -> Instant {
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
fn udp_send() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn_with_handle(udp_sender(n2));
        let t2 = n2.spawn_with_handle(udp_receiver(Some(n1)));
        assert!(t1.await.unwrap() + LATENCY == t2.await.unwrap());
    });
}

#[test]
fn udp_send_early() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn_with_handle(udp_sender(n2)).await;
        let t2 = n2.spawn_with_handle(udp_receiver(Some(n1)));
        assert!(t1.unwrap() + LATENCY == t2.await.unwrap());
    });
}

#[test]
fn udp_send_too_early() {
    Runtime::new().run(|| async {
        let [n1, n2] = setup_net(2).try_into().unwrap();
        let t1 = n1.spawn_with_handle(udp_sender(n2)).await;
        sleep(LATENCY + LATENCY).await;
        let t2 = n2.spawn_with_handle(udp_receiver(None));
        assert!(t1.unwrap() + LATENCY == t2.await.unwrap());
    });
}

#[test]
fn tcp_send() {
    Runtime::new().run(|| async {
        fn msg(i: i32) -> String {
            format!("hello {i}")
        }
        async fn check_tcp_connection(mut tcp: TcpStream, write_num: i32) {
            SimRuntime::timeout(Duration::from_secs(3), async move {
                tcp.write_all(msg(write_num).as_bytes()).await.unwrap();
                let mut buf = [0; 64];
                let read_len = tcp.read(&mut buf).await.unwrap();
                assert_eq!(
                    std::str::from_utf8(&buf[..read_len]).unwrap(),
                    msg(-write_num)
                );
                tcp.close().await.unwrap();
            })
            .await
            .unwrap();
        }
        async fn server(port: u16) {
            let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
                .await
                .unwrap();
            for i in port as i32.. {
                spawn(check_tcp_connection(listener.accept().await.unwrap().0, i)).detach();
            }
        }
        let nodes = setup_net(5);
        let n0 = nodes[0];
        let client = |delay: u64, port: u16, seq: i32| async move {
            sleep(Duration::from_millis(delay)).await;
            let tcp = TcpStream::connect((get_ip(n0), port)).await.unwrap();
            check_tcp_connection(tcp, -(port as i32 + seq)).await;
        };
        n0.spawn(server(100));
        n0.spawn(server(200));
        for i in 0..10 {
            nodes[i % 5].spawn(client(i as u64 * 10, 100, i as i32));
            nodes[(2 + i) % 5].spawn(client(i as u64 * 10, 200, i as i32));
        }
    })
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
