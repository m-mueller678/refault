use refault::{sim_builder::SimBuilder, time::sleep};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    // Define custom start time of simulation
    let simulation_start_time = UNIX_EPOCH + Duration::from_secs(3600 * 24);
    let runtime = SimBuilder::default().with_simulation_start_time(simulation_start_time);

    // Retrieve simulated time at different points in the simulation
    runtime
        .run(move || async move {
            let st1 = SystemTime::now();
            let t1 = Instant::now();
            println!("t1: {st1:?}");

            assert_eq!(simulation_start_time, st1);
            let duration = Duration::from_secs(120);
            sleep(duration).await;

            let st2 = SystemTime::now();
            let t2 = Instant::now();
            println!("t2: {st2:?}");

            assert_eq!(st2.duration_since(st1).unwrap(), duration);
            assert_eq!(t2.checked_duration_since(t1).unwrap(), duration);
        })
        .unwrap();
}
