use deterministic_simulation_core::{runtime::Runtime, time::sleep};
use std::time::Duration;

fn main() {
    Runtime::default().run(|| async {
        sleep(Duration::from_secs(5)).await;
        println!("5 second delay!");
    });
}
