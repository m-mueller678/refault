use chrono::{DateTime, Utc};
use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() {
    // Define custom start time of simulation
    let runtime =
        Runtime::default().with_simulation_start_time(UNIX_EPOCH + Duration::from_secs(3600 * 24));

    // Retrieve simulated time at different points in the simulation
    runtime.run(|| async {
        let now: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now);

        sleep(Duration::from_secs(120)).await;

        let now2: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now2);
    });
}
