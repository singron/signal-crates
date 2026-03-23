#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::AtomicU32;
use loom::thread;
use std::sync::atomic::Ordering;

use signal_lock::{Lock, futex::FutexLock, gettid, pipe::PipeLock};

mod common;
use common::WaitTid;

// This takes about 85 seconds to run.
fn test_loom_signal_impl<L: Lock<AtomicU32> + 'static>() {
    // Take the lock on two threads and perform an intentionally racy increment.
    loom::model(|| {
        loom::stop_exploring();
        let lock = Arc::new(L::new(AtomicU32::new(0)).unwrap());
        let signal_increments = Arc::new(AtomicU32::new(0));
        signal_lock::fake_signal_set_handler({
            let lock = lock.clone();
            let signal_increments = signal_increments.clone();
            move || {
                if let Ok(v) = lock.lock() {
                    v.fetch_add(1, Ordering::Relaxed);
                    signal_increments.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let tid1 = Arc::new(WaitTid::new());

        let thread1 = {
            let lock = lock.clone();
            let tid1 = tid1.clone();
            || {
                thread::spawn(move || {
                    tid1.set(gettid());
                    let atomic = lock.lock().unwrap();
                    // Racy increment if we don't have mutual exclusion.
                    let val = atomic.load(Ordering::Relaxed);
                    atomic.store(val + 1, Ordering::Relaxed);
                })
            }
        };
        let thread2 = {
            let tid1 = tid1.clone();
            || {
                thread::spawn(move || {
                    let tid1 = tid1.wait();
                    signal_lock::fake_signal_thread(tid1);
                })
            }
        };

        loom::explore();
        let thread1 = thread1();
        let thread2 = thread2();
        {
            let atomic = lock.lock().unwrap();
            // Similar racy increment if we don't have mutual exclusion.
            let val = atomic.load(Ordering::Relaxed);
            atomic.store(val + 1, Ordering::Relaxed);
        }
        thread1.join().unwrap();
        thread2.join().unwrap();
        // The signal handler may or may not have interrupted and been able to take the lock, so
        // only consider the increments it actually made.
        let signal_increments = signal_increments.load(Ordering::Relaxed);
        assert!(signal_increments == 0 || signal_increments == 1);
        let val = lock.lock().unwrap().load(Ordering::Relaxed);
        assert_eq!(val, 2 + signal_increments);
    });
}

#[test]
#[cfg(target_os = "linux")]
fn test_loom_signal_futex() {
    test_loom_signal_impl::<FutexLock<AtomicU32>>();
}

#[test]
fn test_loom_signal_pipe() {
    test_loom_signal_impl::<PipeLock<AtomicU32>>();
}
