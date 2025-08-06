use chrono::{DateTime, Utc};
use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::{Duration, SystemTime};

fn main() {
    // Define custom start time of simulation
    let runtime = Runtime::default().with_simulation_start_time(1893456000000);

    // Retrieve simulated time at different points in the simulation
    runtime.run(async {
        let now: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now);

        sleep(Duration::from_secs(120)).await;

        let now2: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now2);
    });
}
