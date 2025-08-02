use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::Duration;

fn main() {
    // Use 'run' instead of 'simulate' in order to actually wait the specified duration
    Runtime::default().run(async {
        sleep(Duration::from_secs(5)).await;
        println!("5 second delay!");
    });
}
