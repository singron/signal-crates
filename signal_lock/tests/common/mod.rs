// This file is included in multiple tests, so some of it is often unused.
#![allow(unused)]

use signal_lock::Tid;
use std::sync::atomic::Ordering;

#[cfg(loom)]
use loom::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicI32},
    },
    thread,
};
#[cfg(not(loom))]
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicI32},
    },
    thread,
};

#[cfg(loom)]
pub use loom::model as loom_model;

#[cfg(not(loom))]
pub fn loom_model<F: Fn() + Sync + Send + 'static>(f: F) {
    f()
}

#[cfg(not(loom))]
pub fn explore() {}

#[cfg(not(loom))]
pub fn stop_exploring() {}

#[cfg(loom)]
pub use loom::{explore, stop_exploring};

// We often need to get the tid from a thread at the beginning of a test, so let's make that
// easier.
pub struct WaitTid {
    mutex: Mutex<Tid>,
    condvar: Condvar,
}

impl WaitTid {
    pub fn new() -> WaitTid {
        WaitTid {
            mutex: Mutex::new(0),
            condvar: Condvar::new(),
        }
    }

    pub fn set(&self, val: Tid) {
        assert_ne!(val, 0);
        *self.mutex.lock().unwrap() = val;
        self.condvar.notify_all();
    }

    pub fn wait(&self) -> Tid {
        let mut g = self.mutex.lock().unwrap();
        while *g == 0 {
            g = self.condvar.wait(g).unwrap();
        }
        *g
    }

    pub fn wait_drop(arc: Arc<WaitTid>) -> Tid {
        return arc.wait();
    }
}

// The loom::sync::Barrier is a non-functional stub implementation.

#[cfg(loom)]
pub type Barrier = SpinBarrier;
#[cfg(not(loom))]
pub type Barrier = CondvarBarrier;

pub struct CondvarBarrier {
    threads: u32,
    mutex: Mutex<BarrierState>,
    condvar: Condvar,
}

struct BarrierState {
    count: u32,
    generation: bool,
}

impl CondvarBarrier {
    pub fn new(threads: u32) -> Self {
        Self {
            threads,
            mutex: Mutex::new(BarrierState {
                count: threads,
                generation: false,
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn wait(&self) {
        let mut g = self.mutex.lock().unwrap();
        let generation = g.generation;
        assert_ne!(g.count, 0);
        g.count -= 1;
        if g.count == 0 {
            g.generation = !generation;
            g.count = self.threads;
            self.condvar.notify_all();
            return;
        }

        loop {
            g = self.condvar.wait(g).unwrap();
            if g.generation != generation {
                break;
            }
            #[cfg(loom)]
            loom::skip_branch();
        }
    }
}

/// This barrier works really well with loom, but the busy looping makes it bad for general use.
pub struct SpinBarrier {
    threads: u32,
    count: AtomicI32,
    generation: AtomicBool,
}

impl SpinBarrier {
    pub fn new(threads: u32) -> Self {
        assert!(threads <= i32::MAX as u32);
        Self {
            threads,
            generation: AtomicBool::new(false),
            count: AtomicI32::new(threads as i32),
        }
    }

    pub fn wait(&self) {
        let generation = self.generation.load(Ordering::SeqCst);
        let count = self.count.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(count > 0);
        let count = count as u32;
        if count == 1 {
            self.count.fetch_add(self.threads as i32, Ordering::SeqCst);
            self.generation.store(!generation, Ordering::SeqCst);
        } else {
            // This part works really well on loom. This discards all states except the one where
            // we yield once and wakeup after the generation has flipped.
            thread::yield_now();
            while generation == self.generation.load(Ordering::SeqCst) {
                #[cfg(loom)]
                loom::skip_branch();

                thread::yield_now();
            }
        }
    }
}
