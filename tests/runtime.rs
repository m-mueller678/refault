use std::sync::atomic::Ordering::Relaxed;
use std::{collections::HashMap, sync::atomic::AtomicUsize, time::Duration};

use deterministic_simulation_core::{
    runtime::{Id, Runtime},
    time::sleep,
};

#[test]
fn id_hashmap() {
    let rt = Runtime::new();
    rt.check_determinism(2, || async {
        let map: HashMap<_, _> = (0..10).map(|i| (Id::new(), i)).collect();
        for &x in map.values() {
            sleep(Duration::from_secs(x)).await;
        }
    })
}

#[test]
#[should_panic]
fn global_state() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    Runtime::new().check_determinism(2, || async {
        sleep(Duration::from_secs(
            1 + COUNTER.fetch_add(1, Relaxed) as u64,
        ))
        .await;
    });
}
