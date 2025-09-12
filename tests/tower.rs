#![cfg(feature = "tower")]
use std::time::{Duration, Instant};

use futures::{StreamExt, stream::FuturesUnordered};
use refault::{
    packet_network::{Addr, ConNet, perfect_connectivity},
    runtime::{Id, NodeId, Runtime},
    simulator::add_simulator,
    time::sleep,
    tower::{Client, run_server},
};
use tower::{Service, service_fn};

const COMPUTE_LATENCY: Duration = Duration::from_millis(50);
const NET_LATENCY: Duration = Duration::from_millis(20);

#[test]
fn tower() {
    Runtime::new().check_determinism(2, || async move {
        add_simulator(ConNet::new(perfect_connectivity(NET_LATENCY)));
        let server = NodeId::create_node();
        let port = Id::new();
        spawn_server(server, port);
        sleep(Duration::from_millis(1)).await;
        NodeId::create_node().spawn(run_client(Addr { node: server, port }, vec![Ok(3), Err(2)]));
    });
}

fn spawn_server(node: NodeId, port: Id) {
    node.spawn(async move {
        run_server(
            port,
            service_fn(move |x: Result<i32, i32>| async move {
                sleep(COMPUTE_LATENCY).await;
                x
            }),
        )
        .await
    })
}

async fn run_client(addr: Addr, inputs: Vec<Result<i32, i32>>) {
    let mut client = Client::new(addr);
    let futures = FuturesUnordered::new();
    let t1 = Instant::now();
    for x in inputs {
        let fut = client.call(x);
        futures.push(async move {
            assert_eq!(fut.await.map_err(|x| x.unwrap_left()), x);
            assert_eq!(t1.elapsed(), COMPUTE_LATENCY + 2 * NET_LATENCY);
        });
    }
    futures.collect::<Vec<()>>().await;
}
