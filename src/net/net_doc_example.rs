# use refault::{
#     net::{Addr, Net, Packet, Socket, perfect_connectivity},
#     NodeId, SimBuilder,
#     id::Id,
#     simulator::{add_simulator,SimulatorHandle},
#     time::sleep,
# };
# use std::time::Duration;
# 
# fn main() {
#     SimBuilder::new_test().run(|| async move {
struct MyPacket(String);
impl Packet for MyPacket {}
add_simulator(Net::new(perfect_connectivity(Duration::from_millis(20))));
let addr1 = Addr {
    port: Id::new(),
    node: NodeId::create_node(),
};
let addr2 = Addr {
    port: Id::new(),
    node: NodeId::create_node(),
};

addr1.node.spawn(async move {
    let socket = Socket::<MyPacket>::open(addr1.port).unwrap();
    let (ping, src_addr) = socket.receive().await.unwrap();
    assert!(ping.0 == "ping");
    assert!(src_addr == addr2);
    socket.send(addr2, MyPacket("pong".into())).await.unwrap();
    sleep(Duration::from_millis(1)).await;

    // sending packets does not require an open socket, only receiving.
    let other_src_port = Id::new();
    SimulatorHandle::<Net>::get()
        .with(|net| net.send(other_src_port, addr2, MyPacket("pong2".into())))
        .await
        .unwrap();
});

addr2.node.spawn(async move {
    let socket = Socket::<MyPacket>::open(addr2.port).unwrap();
    // ensure socket on node 1 is opened before we send the packet
    sleep(Duration::from_millis(1)).await;
    socket.send(addr1, MyPacket("ping".into())).await.unwrap();
    assert!(socket.receive().await.unwrap().0.0 == "pong");
    assert!(socket.receive().await.unwrap().0.0 == "pong2");
});
#     }).unwrap();
# }
