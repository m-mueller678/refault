use std::time::Duration;

use refault::{runtime::Runtime, time::sleep};

fn main() {
    Runtime::default().run(|| async {
        sleep(Duration::from_secs(5)).await;
        println!("5 second delay!");
    });
}
