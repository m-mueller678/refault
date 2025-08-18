use deterministic_simulation_core::{
    runtime::{Id, NodeId, Runtime, spawn},
    tarpc::listen,
    time::sleep,
};
use futures::StreamExt;
use std::{future::ready, time::Duration};
use tarpc::server::{BaseChannel, Channel};

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
        let server = NodeId::create_node();
        let server_port = Id::new();
        server.spawn(async move {
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
        });
    })
}
