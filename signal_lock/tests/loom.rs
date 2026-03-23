#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::AtomicU32;
use loom::thread;
use std::sync::atomic::Ordering;

use signal_lock::Lock;

static ONE_TEST_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn test_loom_increment<L: Lock<AtomicU32> + 'static>() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    // Take the lock on two threads and perform an intentionally racy increment.
    loom::model(|| {
        eprintln!("\nStart:");
        let _drop = signal_lock::fake_signal_clear_statics_on_drop();
        let _drop = signal_lock::fake_signal_clear_thread_locals_on_drop();
        let l1 = Arc::new(L::new(AtomicU32::new(0)).unwrap());
        let l2 = l1.clone();
        let handle = thread::spawn(move || {
            let _drop = signal_lock::fake_signal_clear_thread_locals_on_drop();
            let atomic = l1.lock().unwrap();
            let val = atomic.load(Ordering::Relaxed);
            assert!(val == 0 || val == 1);
            atomic.store(val + 1, Ordering::Relaxed);
        });
        {
            let atomic = l2.lock().unwrap();
            let val = atomic.load(Ordering::Relaxed);
            assert!(val == 0 || val == 1);
            atomic.store(val + 1, Ordering::Relaxed);
        }
        handle.join().unwrap();
        {
            let atomic = l2.lock().unwrap();
            let val = atomic.load(Ordering::Relaxed);
            assert_eq!(val, 2);
        }
    });
}

#[test]
#[cfg(target_os = "linux")]
fn test_loom_increment_futex() {
    test_loom_increment::<signal_lock::futex::FutexLock<AtomicU32>>();
}

#[test]
fn test_loom_increment_pipe() {
    test_loom_increment::<signal_lock::pipe::PipeLock<AtomicU32>>();
}
