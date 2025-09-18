use std::cell::Cell;
use std::future::pending;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::{collections::HashMap, sync::atomic::AtomicUsize, time::Duration};

use refault::NodeId;
use refault::executor::{TaskHandle, spawn};
use refault::id::Id;
use refault::simulator::{Simulator, SimulatorHandle, add_simulator};
use refault::{SimBuilder, time::sleep};
use scopeguard::defer;

#[test]
fn node_task_runs() {
    SimBuilder::new().run_test(|| async move {
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();
        NodeId::create_node().spawn(async move {
            flag2.store(true, Relaxed);
        });
        sleep(Duration::from_millis(1)).await;
        assert!(flag.load(Relaxed));
    });
}

#[test]
fn id_hashmap() {
    SimBuilder::new().run_test(|| async {
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
    SimBuilder::new().run_test(|| async {
        sleep(Duration::from_secs(
            1 + COUNTER.fetch_add(1, Relaxed) as u64,
        ))
        .await;
    });
}

#[test]
#[should_panic]
fn spawn_on_drop() {
    SimBuilder::new().run_test(|| async {
        defer! {
            spawn(async move{panic!()}).detach();
        }
        pending::<()>().await;
    });
}

struct Sim;
impl Simulator for Sim {}

#[test]
fn simulator_on_drop() {
    SimBuilder::new().run_test(|| async {
        add_simulator(Sim);
        defer! {
            SimulatorHandle::<Sim>::get().with(|_|{})
        }
    })
}

#[test]
#[should_panic]
fn simulator_after_node() {
    SimBuilder::new().run_test(|| async {
        NodeId::create_node();
        add_simulator(Sim);
    })
}

#[test]
#[should_panic]
fn simulator_after_stop() {
    SimBuilder::new().run_test(|| async {
        defer! {
            add_simulator(Sim);
        }
        pending().await
    })
}

#[test]
fn kill_node() {
    SimBuilder::new().run_test(|| async {
        let rc = Rc::new(Cell::new(0));
        let spawn_on_new_node = || {
            let rc2 = rc.clone();
            let node = NodeId::create_node();
            node.spawn(async move {
                sleep(Duration::from_millis(10)).await;
                rc2.set(1);
                sleep(Duration::from_millis(10)).await;
            });
            node
        };
        let n1 = spawn_on_new_node();
        let n2 = spawn_on_new_node();
        assert!(Rc::strong_count(&rc) == 3);
        n1.stop();
        assert!(Rc::strong_count(&rc) == 2);
        sleep(Duration::from_millis(15)).await;
        n2.stop();
        assert!(Rc::strong_count(&rc) == 1);
        assert!(rc.get() == 1);
    })
}

#[test]
fn abort_self() {
    SimBuilder::new().run_test(|| async {
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
