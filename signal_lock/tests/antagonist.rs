#[cfg(target_os = "linux")]
use signal_lock::futex::FutexLock;
use signal_lock::{Lock, Tid, gettid, pipe::PipeLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Barrier, Mutex};

#[cfg(not(miri))]
unsafe fn setup_handler(
    sig: libc::c_int,
    handler: unsafe extern "C" fn(libc::c_int, *const libc::siginfo_t, *const libc::c_void),
) {
    let mut new: libc::sigaction = unsafe { std::mem::zeroed() };
    new.sa_sigaction = handler as usize;
    // Hard mode: SA_NODEFER means we can interrupt the handler!
    new.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
    unsafe { libc::sigemptyset(&mut new.sa_mask) };
    let res = unsafe { libc::sigaction(sig, &new, std::ptr::null_mut()) };
    assert_eq!(res, 0);
}

#[cfg(miri)]
unsafe fn setup_handler(
    sig: libc::c_int,
    handler: unsafe extern "C" fn(libc::c_int, *const libc::siginfo_t, *const libc::c_void),
) {
    signal_lock::fake_signal_set_handler(move || {
        unsafe { handler(sig, std::ptr::null(), std::ptr::null()) };
    });
}

struct ThreadContext<L: Lock<u64>> {
    local_lock: L,
    signal_count: AtomicU64,
    local_lock_count: AtomicU64,
}

impl<L: Lock<u64>> ThreadContext<L> {
    fn new() -> Self {
        Self {
            local_lock: L::new(0).unwrap(),
            signal_count: AtomicU64::new(0),
            local_lock_count: AtomicU64::new(0),
        }
    }
}

fn hash(i: usize, j: usize) -> u64 {
    use std::hash::Hasher;
    let mut h = std::hash::DefaultHasher::new();
    h.write_usize(i);
    h.write_usize(j);
    h.finish()
}

// Reduce false sharing by wrapping in this.
#[repr(align(128))]
struct CacheAlign<T>(T);

static ONE_TEST_AT_A_TIME: Mutex<()> = Mutex::new(());
static THREAD_CONTEXT_SETUP_LOCK: Mutex<()> = Mutex::new(());

// static items can't be declared with generic parameters in generic functions, so we will do it in
// a macro and pass this struct to the generic function.
struct Statics<L: Lock<u64> + 'static> {
    get_thread_context: fn() -> &'static ThreadContext<L>,
    set_thread_context: unsafe fn(ThreadContext<L>),
    handler: unsafe extern "C" fn(libc::c_int, *const libc::siginfo_t, *const libc::c_void),
}

macro_rules! test_statics {
    ($var:ident, $L:ty) => {
        static THREAD_CONTEXTS: AtomicPtr<HashMap<Tid, *mut ThreadContext<$L>>> =
            AtomicPtr::new(std::ptr::null_mut());

        fn get_thread_context() -> &'static ThreadContext<$L> {
            let tid = gettid();
            let map = unsafe { &*THREAD_CONTEXTS.load(Ordering::SeqCst) };
            unsafe { &**map.get(&tid).unwrap() }
        }

        // Safety: Cannot be called while any thread calls get_thread_context.
        unsafe fn set_thread_context(ctx: ThreadContext<$L>) {
            let tid = gettid();
            let ptr = Box::leak(Box::new(ctx)) as *mut _;
            let _g = THREAD_CONTEXT_SETUP_LOCK.lock().unwrap();
            let mut map_ptr = THREAD_CONTEXTS.load(Ordering::SeqCst);
            if map_ptr.is_null() {
                map_ptr = Box::leak(Box::new(HashMap::new())) as *mut _;
                THREAD_CONTEXTS.store(map_ptr, Ordering::SeqCst);
            }
            let map = unsafe { &mut *map_ptr };
            map.insert(tid, ptr);
        }

        unsafe extern "C" fn handler(
            _sig: libc::c_int,
            _info: *const libc::siginfo_t,
            _ucontext: *const libc::c_void,
        ) {
            let ctx = get_thread_context();

            ctx.signal_count.fetch_add(1, Ordering::Relaxed);
            let res = ctx.local_lock.lock();
            // Err(Recursive) happens if we interrupt while lock is held.
            if let Ok(mut guard) = res {
                let n = *guard;
                unsafe { libc::sched_yield() };
                *guard = n + 1;
                ctx.local_lock_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        let $var = Statics {
            get_thread_context,
            set_thread_context,
            handler,
        };
    };
}

fn test_impl<L: Lock<u64>>(statics: Statics<L>) {
    unsafe { setup_handler(libc::SIGUSR1, statics.handler) };

    let small = cfg!(miri);
    let n_lock_threads = if small { 3 } else { 24 };
    let n_rounds = if small { 100 } else { 1000000 };
    let n_signals = if small { 10 } else { 1000 };
    let n_global_locks = 1.max(n_lock_threads / 4);
    // After the first wait of this barrier, it's safe to call get_thread_context since there are
    // no more calls to set_thread_context.
    // After the second wait, all threads are done working.
    let barrier = &Barrier::new(n_lock_threads + 1);

    let tids = &Mutex::new(Vec::<Tid>::with_capacity(n_lock_threads));

    let global_locks: Vec<CacheAlign<L>> = (0..n_global_locks)
        .map(|_| CacheAlign(L::new(0).unwrap()))
        .collect();
    let global_locks = &global_locks;

    std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for thread_idx in 0..n_lock_threads {
            threads.push(scope.spawn(move || {
                let ctx = ThreadContext::new();
                unsafe { (statics.set_thread_context)(ctx) };
                {
                    tids.lock().unwrap().push(gettid());
                }

                barrier.wait();

                let ctx = (statics.get_thread_context)();
                for round_idx in 0..n_rounds {
                    // .lock() should always succeed from non-signal-handler.
                    let mut local_lock = ctx.local_lock.lock().unwrap();
                    let local_lock_read = *local_lock;
                    let lock_idx = (hash(thread_idx, round_idx) as usize) % n_global_locks;

                    {
                        let mut global_lock = global_locks[lock_idx].0.lock().unwrap();
                        let global_lock_read = *global_lock;
                        unsafe { libc::sched_yield() };
                        *global_lock = global_lock_read + 1;
                    }

                    *local_lock = local_lock_read + 1;
                    ctx.local_lock_count.fetch_add(1, Ordering::Relaxed);
                }
                barrier.wait();

                ctx
            }));
        }
        let pid = unsafe { libc::getpid() };
        barrier.wait();
        let tids = &*tids.lock().unwrap();

        // Antagonize! send signals to threads.
        for _ in 0..n_signals {
            for tid in tids {
                tgkill(pid, *tid as libc::pid_t, libc::SIGUSR1);
            }
            unsafe { libc::sched_yield() };
        }

        barrier.wait();

        for join in threads {
            let ctx = join.join().unwrap();
            let cnt = *ctx.local_lock.lock().unwrap();
            assert_eq!(cnt, ctx.local_lock_count.load(Ordering::SeqCst));
            assert!(n_signals >= ctx.signal_count.load(Ordering::SeqCst));
            assert!(cnt >= n_rounds as u64);
            assert!(cnt <= n_rounds as u64 + n_signals);
        }

        let mut total_global_locks = 0;
        for l in global_locks {
            total_global_locks += *l.0.lock().unwrap();
        }
        assert_eq!(total_global_locks, (n_lock_threads * n_rounds) as u64);
    });
}

#[test]
#[cfg_attr(loom, ignore)]
#[cfg(target_os = "linux")]
fn test_futex() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    test_statics!(statics, FutexLock<u64>);
    test_impl::<FutexLock<u64>>(statics);
}

#[test]
#[cfg_attr(loom, ignore)]
fn test_pipe() {
    let _g = ONE_TEST_AT_A_TIME.lock().unwrap();
    test_statics!(statics, PipeLock<u64>);
    test_impl::<PipeLock<u64>>(statics);
}

#[cfg(all(not(miri), target_os = "linux", target_env = "gnu"))]
fn tgkill(pid: libc::pid_t, tid: libc::pid_t, sig: libc::c_int) -> libc::c_int {
    unsafe { libc::tgkill(pid, tid, sig) }
}

#[cfg(all(not(miri), target_os = "linux", not(target_env = "gnu")))]
fn tgkill(pid: libc::pid_t, tid: libc::pid_t, sig: libc::c_int) -> libc::c_int {
    unsafe { libc::syscall(libc::SYS_tgkill, pid, tid, sig) as i32 }
}

#[cfg(all(not(miri), target_os = "freebsd"))]
fn tgkill(pid: libc::pid_t, tid: libc::pid_t, sig: libc::c_int) -> libc::c_int {
    // thr_kill is limited to the current processs (unlike tkill) so it's safe to use, but we'll
    // use thr_kill2 for consistency.
    //
    // On FreeBSD, thread id is 64-bit in kernel ABIs for backwards compatibility, but it's
    // actually 32 bit (pthread_getthreadid_np returns 32 bit int).
    unsafe { libc::thr_kill2(pid, tid.into(), sig) }
}

#[cfg(miri)]
fn tgkill(_pid: libc::pid_t, tid: libc::pid_t, _sig: libc::c_int) -> libc::c_int {
    signal_lock::fake_signal_thread(tid as Tid);
    0
}
