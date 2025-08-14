use std::{collections::HashMap, time::Duration};

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
