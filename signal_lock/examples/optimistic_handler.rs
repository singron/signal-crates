// In this example, the signal handler optimistically tries to take the lock and perform an action,
// but if the attempt returns Err(Recursive), then it sets SIGNAL_DEFERRED. The non-signal-hanlder
// checks SIGNAL_DEFERRED after the critical section.
//
// This is an alternative to the signal handler always only setting a boolean variable. Then the
// program has to check for this boolean variable regularly, potentially in many different
// locations in the code.
use signal_lock::{SignalLock, SignalLockError};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

struct Object;

// SignalLock::new isn't const-fn, so we will initialize in main.
static LOCK: AtomicPtr<SignalLock<Object>> = AtomicPtr::new(std::ptr::null_mut());
static SIGNAL_DEFERRED: AtomicBool = AtomicBool::new(false);

fn get_lock() -> &'static SignalLock<Object> {
    let ptr = LOCK.load(Ordering::Relaxed);
    // Safety: called after initialization in main.
    unsafe { &*ptr }
}

fn write_stderr(msg: &[u8]) {
    unsafe { libc::write(libc::STDERR_FILENO, msg.as_ptr() as *const _, msg.len()) };
}

unsafe extern "C" fn signal_handler(_sig: libc::c_int) {
    let saved_err = errno::errno();
    match get_lock().lock() {
        Ok(g) => {
            write_stderr(b"Handling in signal handler\n");
            do_the_thing(&g);
        },
        Err(SignalLockError::Recursive) =>
        // Tell our thread to do it as soon as it unlocks:
        {
            write_stderr(b"Deferring in signal handler\n");
            SIGNAL_DEFERRED.store(true, Ordering::Relaxed)
        }
    }
    errno::set_errno(saved_err);
}

fn normal_code(raise_in_cs: bool) {
    {
        let g = get_lock().lock().unwrap();
        // Do something with g ...
        let _ = *g;
        if raise_in_cs {
            unsafe { libc::raise(libc::SIGUSR1) };
        }
    }

    // We got a signal while we were in the critical section. Let's handle it now.
    while SIGNAL_DEFERRED.swap(false, Ordering::Relaxed) {
        let g = get_lock().lock().unwrap();
        eprintln!("Detected deferred signal after critical section");
        do_the_thing(&g);
    }

    // Outside of the critical section, our signal handler will be able to call do_the_thing and we
    // don't have to check SIGNAL_DEFERRED.
    std::thread::sleep(std::time::Duration::from_millis(10));
}

fn do_the_thing(_: &Object) {}

fn main() {
    let lock_ptr = Box::leak(Box::new(SignalLock::new(Object).unwrap())) as *mut _;
    LOCK.store(lock_ptr, Ordering::Relaxed);

    // Register handler.
    let mut new: libc::sigaction = unsafe { std::mem::zeroed() };
    new.sa_sigaction = signal_handler as *const () as libc::sighandler_t;
    unsafe { libc::sigemptyset(&mut new.sa_mask) };
    let res = unsafe { libc::sigaction(libc::SIGUSR1, &new, std::ptr::null_mut()) };
    assert_eq!(res, 0);

    normal_code(true);
    // prints:
    // Deferring in signal handler
    // Detected deferred signal after critical section

    unsafe { libc::raise(libc::SIGUSR1) };
    // prints:
    // Handling in signal handler

    normal_code(false);
    // prints nothing.
}
