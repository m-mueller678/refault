use std::time::Duration;

use refault::{sim_builder::SimBuilder, time::sleep};

fn main() {
    SimBuilder::default()
        .run(|| async {
            sleep(Duration::from_secs(5)).await;
            println!("5 second delay!");
        })
        .unwrap();
}
