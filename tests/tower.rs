use std::time::Duration;

use refault::{
    packet_network::{ConNet, perfect_connectivity},
    runtime::{Id, NodeId, Runtime},
    simulator::add_simulator,
    time::sleep,
    tower::run_server,
};
use tower::service_fn;

const COMPUTE_LATENCY: Duration = Duration::from_millis(50);
const NET_LATENCY: Duration = Duration::from_millis(20);

#[test]
fn tower() {
    Runtime::new().check_determinism(2, || async move {
        add_simulator(ConNet::new(perfect_connectivity(NET_LATENCY)));
        let n0 = NodeId::create_node();
        let p0 = Id::new();
        spawn_server(n0, p0);
    });
}

fn spawn_server(n0: NodeId, p0: Id) {
    n0.spawn(async move {
        run_server(
            p0,
            service_fn(move |x: Result<i32, i32>| async move {
                sleep(COMPUTE_LATENCY).await;
                x
            }),
        )
        .await
    })
}
