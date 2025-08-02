use std::collections::HashSet;
use deterministic_simulation_core::network::{listen, send};
use deterministic_simulation_core::node::{current_node, Node};
use deterministic_simulation_core::task::{spawn};
use deterministic_simulation_core::time::sleep;
use std::time::{Duration, SystemTime};
use deterministic_simulation_core::context::simulate;
use deterministic_simulation_core::event::check_determinism;
use human_duration::human_duration;
use rand::Rng;

fn main() {
    simulate(hash_map_test());
}

#[allow(dead_code)]
async fn agnostik_test() {
    let handle1 = agnostik::spawn(calculate_async());
    let handle2 = agnostik::spawn(calculate_async());
    println!("{}", handle1.await);
    println!("{}", handle2.await);
}

#[allow(dead_code)]
async fn time_test() {
    let now = SystemTime::now();
    println!("{}", human_duration(&now.duration_since(SystemTime::UNIX_EPOCH).unwrap()));

    sleep(Duration::from_secs(1)).await;
    
    let now2 = SystemTime::now();
    println!("{}", human_duration(&now2.duration_since(SystemTime::UNIX_EPOCH).unwrap()));
}

#[allow(dead_code)]
async fn rand_test() {
    let mut rng = rand::rng();
    println!("{}", rng.random_range(0..100));
}

#[allow(dead_code)]
async fn getrandom_test() {
    println!("{}", getrandom::u32().unwrap());
}

#[allow(dead_code)]
async fn hash_map_test() {
    let mut set: HashSet<usize> = HashSet::default();
    for i in 0..5 {
        set.insert(i);
    }
    println!("{:?}", set);
}

#[allow(dead_code)]
async fn spawn_result() {
    let fut1 = spawn(calculate_async());
    let fut2 = spawn(calculate_async());

    let result1 = fut1.await;
    println!("1 Ready");
    let result2 = fut2.await;
    println!("2 Ready");
    println!("{result1}  {result2}");
}

async fn calculate_async() -> i32 {
    sleep(Duration::from_secs(5)).await;
    5
}

#[allow(dead_code)]
async fn print_numbers() {
    Node::new().spawn(async {
        spawn(print_numbers_delayed(1));
        spawn(print_numbers_delayed(1));
        spawn(print_numbers_delayed(1));
        spawn(print_numbers_delayed(1));
        spawn(print_numbers_delayed(1));
        spawn(print_numbers_delayed(1));
        print_numbers_delayed(1).await;
    });
    Node::new().spawn(print_numbers_delayed(1));
    Node::new().spawn(print_numbers_delayed(2));
}

#[allow(dead_code)]
async fn network_sample() {
    let receiver_node = Node::new();
    let receiver_node_id = receiver_node.id;
    receiver_node.spawn(async {
        loop {
            let package = listen().await;
            println!("{}", package.message);
        }
    });

    Node::new().spawn(async move {
        send(String::from("message 1"), receiver_node_id);
        sleep(Duration::from_millis(500)).await;
        send(String::from("message 2"), receiver_node_id);
    }).await;
}

async fn print_numbers_delayed(delay_seconds: u64) {
    for i in 1..=5 {
        sleep(Duration::from_secs(delay_seconds)).await;
        let current_node_id = current_node().unwrap();
        println!("{current_node_id}: {i}");
    }
}