#[cfg(loom)]
use loom::sync::atomic::AtomicU32;
#[cfg(loom)]
use loom::sync::{Condvar, Mutex};
#[cfg(not(loom))]
use std::sync::atomic::AtomicU32;
/// This futex implementation is NOT signal safe, but it should only be used with loom where we are
/// going to fake signals.
///
/// Note that miri has a built-in futex shim and does not need to use this.
use std::sync::atomic::Ordering;
#[cfg(not(loom))]
use std::sync::{Condvar, Mutex};

pub(crate) struct Futex {
    word: AtomicU32,
    mutex: Mutex<()>,
    cv: Condvar,
}

impl Futex {
    pub(crate) fn new(val: u32) -> Futex {
        Futex {
            word: AtomicU32::new(val),
            mutex: Mutex::new(()),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn atomic(&self) -> &AtomicU32 {
        &self.word
    }

    pub(crate) fn wait(&self, val: u32) {
        let mut g = self.mutex.lock().unwrap();
        if self.word.load(Ordering::Relaxed) != val {
            return;
        }
        g = self.cv.wait(g).unwrap();
        drop(g);
        // Note spurious wakeups are allowed.
    }

    pub(crate) fn wake_one(&self) {
        let _g = self.mutex.lock().unwrap();
        self.cv.notify_one();
    }
}
