use deterministic_simulation_core::executor::spawn;
use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::Duration;

fn main() {
    Runtime::default().run(async {
        // Spawn multiple futures for simultaneous execution
        let future1 = spawn(calculate_async());
        let future2 = spawn(calculate_async());

        let val1 = future1.await;
        let val2 = future2.await;

        println!("{}", val1 + val2);
    });
}

async fn calculate_async() -> u32 {
    sleep(Duration::from_secs(5)).await;
    5
}
