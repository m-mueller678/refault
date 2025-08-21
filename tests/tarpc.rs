use deterministic_simulation_core::{
    packet_network::Addr,
    packet_network::{PerfectConnectivity, network_from_connectivity},
    runtime::{Id, NodeId, Runtime, spawn},
    simulator::add_simulator,
    tarpc::{ClientConnection, listen},
    time::sleep,
};
use futures::{StreamExt, stream::FuturesUnordered};
use std::{future::ready, time::Duration};
use tarpc::{
    client::Config,
    context::Context,
    server::{BaseChannel, Channel},
};

#[tarpc::service]
pub trait Greeter {
    async fn hello(name: String) -> String;
}

#[derive(Clone)]
struct HelloGreeter(u64);

impl Greeter for HelloGreeter {
    async fn hello(self, _: tarpc::context::Context, name: String) -> String {
        sleep(Duration::from_secs(self.0)).await;
        format!("Hello, {name}!")
    }
}

#[test]
fn hello() {
    Runtime::new().run(|| async {
        add_simulator(network_from_connectivity(PerfectConnectivity::new(
            Duration::from_millis(1),
        )));
        let server = NodeId::create_node();
        let server_port = Id::new();
        server
            .spawn(async move {
                listen(server_port)
                    .unwrap()
                    .filter_map(|r| ready(r.ok()))
                    .map(BaseChannel::with_defaults)
                    .map(|channel| {
                        let server = HelloGreeter(1);
                        channel
                            .execute(server.serve())
                            .for_each(|x| async move { spawn(x).await.unwrap() })
                    })
                    .buffer_unordered(10)
                    .for_each(|_| async {})
                    .await;
            })
            .detach();
        let client = NodeId::create_node();
        client
            .spawn(async move {
                let connection = ClientConnection::new(Addr {
                    node: server,
                    port: server_port,
                })
                .await
                .unwrap();
                let client = GreeterClient::new(Config::default(), connection);
                spawn(client.dispatch).detach();
                let client = &client.client;
                (0..100)
                    .map(|i| async move {
                        client
                            .hello(Context::current(), format!("c{i}"))
                            .await
                            .unwrap();
                    })
                    .collect::<FuturesUnordered<_>>()
                    .collect::<()>()
                    .await;
            })
            .detach();
    })
}
