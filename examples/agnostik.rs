use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::Duration;

fn main() {
    Runtime::default().simulate(async {
        let handle1 = agnostik::spawn(calculate_async());
        let handle2 = agnostik::spawn(calculate_async());
        println!("{}", handle1.await);
        println!("{}", handle2.await);
    });
}

async fn calculate_async() -> u32 {
    sleep(Duration::from_secs(5)).await;
    5
}
