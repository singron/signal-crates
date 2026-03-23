use std::sync::atomic::Ordering;
mod common;
use common::{Barrier, explore, loom_model, stop_exploring};
#[cfg(loom)]
use loom::{
    sync::{Arc, atomic::AtomicU32},
    thread::spawn,
};
#[cfg(not(loom))]
use std::{
    sync::{Arc, atomic::AtomicU32},
    thread::spawn,
};

static ONE_TEST_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_barrier2() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    loom_model(|| {
        stop_exploring();
        let b = Arc::new(Barrier::new(2));
        let v1 = Arc::new(AtomicU32::new(0));
        let v2 = Arc::new(AtomicU32::new(0));
        let handle = spawn({
            let b = b.clone();
            let v1 = v1.clone();
            let v2 = v2.clone();
            explore();
            move || {
                // Intentionally racy increments protected by the barrier.
                v1.store(1 + v1.load(Ordering::Relaxed), Ordering::Relaxed);
                b.wait();
                v2.store(1 + v2.load(Ordering::Relaxed), Ordering::Relaxed);
            }
        });
        v2.store(1 + v2.load(Ordering::Relaxed), Ordering::Relaxed);
        b.wait();
        v1.store(1 + v1.load(Ordering::Relaxed), Ordering::Relaxed);
        handle.join().unwrap();
        drop(b);
        let v1 = Arc::try_unwrap(v1).unwrap();
        let v2 = Arc::try_unwrap(v2).unwrap();
        assert_eq!(2, v1.load(Ordering::Relaxed));
        assert_eq!(2, v2.load(Ordering::Relaxed));
    });
}
