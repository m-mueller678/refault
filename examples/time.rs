use chrono::{DateTime, Utc};
use deterministic_simulation_core::runtime::Runtime;
use deterministic_simulation_core::time::sleep;
use std::time::{Duration, SystemTime};

fn main() {
    // Define custom start time of simulation
    let runtime = Runtime {
        simulation_start_time: 1893456000000, // 2030
        ..Default::default()
    };

    // Retrieve simulated time at different points in the simulation
    runtime.simulate(async {
        let now: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now);

        sleep(Duration::from_secs(120)).await;

        let now2: DateTime<Utc> = SystemTime::now().into();
        println!("{}", now2);
    });
}
