use deterministic_simulation_core::executor::spawn;
use deterministic_simulation_core::network::{Network, NetworkPackage, listen, send};
use deterministic_simulation_core::node::{Node, current_node, get_node};
use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;

struct CustomNetwork {}

// Custom implementation for Network with random transmission delay
impl Network for CustomNetwork {
    fn transmit_message(&self, package: NetworkPackage) {
        let mut rng = rand::rng();
        let delay = rng.random_range(20..100);

        spawn(async move {
            sleep(Duration::from_millis(delay)).await;
            get_node(&current_node().unwrap()).receive_message(package);
        });
    }
}

fn main() {
    let runtime = Runtime::default().with_network(Arc::new(CustomNetwork {}));

    // Simulate two nodes communicating over the network
    runtime.run(async {
        let receiver_node = Node::new();
        let receiver_node_id = receiver_node.id;

        receiver_node.spawn(async {
            loop {
                let package = listen().await;
                let message = package.message.downcast::<String>().unwrap();
                println!("Receiver node received: '{message}'");
            }
        });

        Node::new()
            .spawn(async move {
                send(Box::new(String::from("message 1")), receiver_node_id);
                sleep(Duration::from_millis(500)).await;
                send(Box::new(String::from("message 2")), receiver_node_id);
            })
            .await;
    });
}
