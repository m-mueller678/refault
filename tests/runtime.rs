use std::cell::Cell;
use std::future::pending;
use std::rc::Rc;
use std::sync::atomic::Ordering::Relaxed;
use std::{collections::HashMap, sync::atomic::AtomicUsize, time::Duration};

use deterministic_simulation_core::runtime::{TaskHandle, spawn};
use deterministic_simulation_core::{
    runtime::{Id, Runtime},
    time::sleep,
};
use scopeguard::defer;

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

#[test]
fn spawn_on_drop() {
    Runtime::new().run(|| async {
        defer! {
            spawn(async move{panic!()});
        }
        pending::<()>().await;
    });
}

#[test]
fn abort_self() {
    Runtime::new().run(|| async {
        let handle = Rc::new(Cell::new(Option::<TaskHandle<()>>::None));
        let h2 = handle.clone();
        let task = spawn(async move {
            sleep(Duration::from_millis(1)).await;
            h2.take().unwrap().abort();
            pending::<()>().await;
        });
        handle.set(Some(task));
        sleep(Duration::from_millis(2)).await;
        assert!(Rc::strong_count(&handle) == 1);
    })
}
