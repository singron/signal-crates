#![cfg(any(loom, miri))]

use signal_lock::{
    fake_signal_check_interrupt, fake_signal_clear_statics_on_drop,
    fake_signal_clear_thread_locals_on_drop, fake_signal_thread, gettid,
};
use std::sync::atomic::Ordering;

#[cfg(loom)]
use loom::thread::{JoinHandle, spawn};
#[cfg(not(loom))]
use std::thread::{JoinHandle, spawn};

#[cfg(loom)]
use loom::sync::{Arc, atomic::AtomicU32};
#[cfg(not(loom))]
use std::sync::{Arc, atomic::AtomicU32};

mod common;
use common::{Barrier, WaitTid, explore, loom_model, stop_exploring};

static ONE_TEST_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_fake_signal_self() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    loom_model(|| {
        stop_exploring();
        let _drop = fake_signal_clear_statics_on_drop();
        let _drop = fake_signal_clear_thread_locals_on_drop();
        let last_tid = Arc::new(AtomicU32::new(0));

        signal_lock::fake_signal_set_handler({
            let last_tid = last_tid.clone();
            move || {
                last_tid.store(gettid(), Ordering::SeqCst);
            }
        });

        explore();

        // Run new threads to ensure unused tids.

        let other_tid = Arc::new(WaitTid::new());

        let other_handle = {
            let other_tid = other_tid.clone();
            spawn(move || {
                let _drop = fake_signal_clear_thread_locals_on_drop();
                other_tid.set(gettid());
            })
        };

        let handle1 = spawn(move || {
            let _drop = fake_signal_clear_thread_locals_on_drop();
            let tid = gettid();
            assert_ne!(tid, last_tid.load(Ordering::SeqCst));

            fake_signal_check_interrupt();
            assert_ne!(tid, last_tid.load(Ordering::SeqCst));

            // Signaling another thread doesn't signal us.
            fake_signal_thread(other_tid.wait());
            fake_signal_check_interrupt();
            assert_ne!(tid, last_tid.load(Ordering::SeqCst));

            fake_signal_thread(tid);
            assert_ne!(tid, last_tid.load(Ordering::SeqCst));
            fake_signal_check_interrupt();
            assert_eq!(tid, last_tid.load(Ordering::SeqCst));
        });
        handle1.join().unwrap();
        other_handle.join().unwrap();
    });
}

#[test]
fn test_fake_signal() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    loom_model(|| {
        stop_exploring();
        let _drop = fake_signal_clear_statics_on_drop();
        let _drop = fake_signal_clear_thread_locals_on_drop();
        let last_tid = Arc::new(AtomicU32::new(0));

        signal_lock::fake_signal_set_handler({
            let last_tid = last_tid.clone();
            move || {
                let tid = gettid();
                last_tid.store(tid, Ordering::SeqCst);
            }
        });

        let barrier = Arc::new(Barrier::new(3));

        let tid1 = Arc::new(WaitTid::new());
        let tid2 = Arc::new(WaitTid::new());

        let make_thread = |wtid: &Arc<WaitTid>| -> JoinHandle<_> {
            let last_tid = last_tid.clone();
            let barrier = barrier.clone();
            let wtid = wtid.clone();
            spawn(move || {
                let _drop = fake_signal_clear_thread_locals_on_drop();
                wtid.set(gettid());
                drop(wtid);
                let res = std::panic::catch_unwind(|| {
                    let tid = gettid();

                    barrier.wait();
                    fake_signal_check_interrupt();
                    if tid == last_tid.load(Ordering::SeqCst) {
                        return 1;
                    }
                    return 2;
                });
                //eprintln!("Return");
                return res;
            })
        };

        let handle1 = make_thread(&tid1);
        let handle2 = make_thread(&tid2);
        let tid1 = WaitTid::wait_drop(tid1);
        WaitTid::wait_drop(tid2);

        fake_signal_thread(tid1);

        explore();

        barrier.wait();

        let res1 = handle1.join().unwrap();
        let res2 = handle2.join().unwrap();
        drop(barrier);
        drop(last_tid);
        let res1 = res1.unwrap();
        let res2 = res2.unwrap();
        assert_eq!(1, res1);
        assert_eq!(2, res2);
    });
}
