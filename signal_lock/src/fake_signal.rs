/// A fake signal implementation useful for miri and loom. User code can register a handler, send
/// signals to a thread, and check for signal interruptions at certain points.
///
/// Since this is used in tests and the signals aren't real, it uses non-signal-safe functionality
/// as long as it can't be interrupted by fake signals (e.g. Mutex, thread_local).
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::cell::RefCell;

#[cfg(loom)]
use loom::sync::{Arc, Mutex, atomic::AtomicBool};
#[cfg(loom)]
use loom::{lazy_static, thread_local};
#[cfg(not(loom))]
use std::sync::LazyLock;
use std::sync::atomic::Ordering;
#[cfg(not(loom))]
use std::sync::{Arc, Mutex, atomic::AtomicBool};

use crate::{Tid, gettid};

type FakeHandler = dyn Fn() + Send + Sync;

#[cfg(loom)]
lazy_static! {
    static ref HANDLER: std::sync::Mutex<Option<Arc<FakeHandler>>> = std::sync::Mutex::new(None);
}
#[cfg(not(loom))]
static HANDLER: std::sync::Mutex<Option<Arc<FakeHandler>>> = std::sync::Mutex::new(None);

/// Set the fake signal handler
pub fn fake_signal_set_handler<F: Fn() + Send + Sync + 'static>(f: F) {
    #[cfg(loom)]
    let arc: Arc<dyn Fn() + Send + Sync> = Arc::from_std(std::sync::Arc::new(f));
    #[cfg(not(loom))]
    let arc: Arc<dyn Fn() + Send + Sync> = Arc::new(f);
    *HANDLER.lock().unwrap() = Some(arc);
}

/// You should call this in loom tests in order to avoid panic-in-panic.
/// ```
/// # use signal_lock::fake_signal_clear_statics_on_drop;
/// let _drop = fake_signal_clear_statics_on_drop();
/// ```
pub fn fake_signal_clear_statics_on_drop() -> ClearStatics {
    ClearStatics
}

pub struct ClearStatics;

impl Drop for ClearStatics {
    fn drop(&mut self) {
        *HANDLER.lock().unwrap() = None;
        *IS_SIGNALED_MAP.lock().unwrap() = HashMap::default();
    }
}

/// You should call this in loom tests in order to avoid panic-in-panic.
/// ```
/// # use signal_lock::fake_signal_clear_thread_locals_on_drop;
/// let _drop = fake_signal_clear_thread_locals_on_drop();
/// ```
pub fn fake_signal_clear_thread_locals_on_drop() -> ClearThreadLocals {
    ClearThreadLocals
}

pub struct ClearThreadLocals;

impl Drop for ClearThreadLocals {
    fn drop(&mut self) {
        THREAD_IS_SIGNALED.with(|c| c.take());
    }
}

#[cfg(loom)]
lazy_static! {
    /// Map of TID to whether thread has a pending signal.
    static ref IS_SIGNALED_MAP: Mutex<HashMap<Tid, Arc<AtomicBool>>> = Mutex::default();
}
#[cfg(not(loom))]
static IS_SIGNALED_MAP: std::sync::LazyLock<Mutex<HashMap<Tid, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::default());

// RefCell isn't safe to use with real signal handlers since
// 1. It doesn't use compiler_fence to ensure the borrow flag is written before access to the inner
//    value at the instruction level. The compiler is free to reorder these as long as the write
//    happens before the next apparent read of the flag.
// 2. The compiler might decide nothing reads the borrow flag while it's borrowed and elimite the
//    write completely. I.e. the write isn't volatile.
// 3. The borrow bookeeping isn't atomic at the instruction level. A signal can interrupt
//    between the instruction that checks the value isn't borrowed and the instruction that marks
//    the value as borrowed.
// 
// All of these aren't problems with our fake signal implementation since the call to the signal
// handler is visible to the optimizer.
thread_local! {
    static THREAD_IS_SIGNALED: RefCell<Option<Arc<AtomicBool>>> = Some(get_thread_is_signaled()).into();
}

fn get_thread_is_signaled() -> Arc<AtomicBool> {
    let tid = gettid();
    let mut map = IS_SIGNALED_MAP.lock().unwrap();
    let entry = map.entry(tid);
    entry
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

/// Check for a pending signal for this thread and run the handler.
pub fn fake_signal_check_interrupt() {
    if THREAD_IS_SIGNALED.with(|s| s.borrow().as_ref().unwrap().swap(false, Ordering::Relaxed)) {
        handle_fake_signal();
    }
}

fn handle_fake_signal() {
    let handler;
    {
        handler = HANDLER.lock().unwrap().clone();
    }
    if let Some(handler) = handler {
        handler();
    }
}

/// Set a pending signal for a thread.
pub fn fake_signal_thread(tid: Tid) {
    let mut map = IS_SIGNALED_MAP.lock().unwrap();
    let entry = map.entry(tid);
    match entry {
        Entry::Occupied(e) => e.get().store(true, Ordering::Relaxed),
        Entry::Vacant(e) => {
            e.insert(Arc::new(AtomicBool::new(true)));
        }
    };
}
