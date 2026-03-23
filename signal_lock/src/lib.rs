#[cfg(not(loom))]
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

// cfg_select! would be convenient here, but it was only recently stabalized for Rust 1.94.

#[cfg(target_os = "linux")]
pub mod futex;

#[cfg(target_family = "unix")]
pub mod pipe;

pub type LockNewError = pipe::PipeNewError;

/// This trait is a convenience to users to write generic code over multiple lock implementations.
pub trait Lock<T>: Sized + Sync + Send {
    type Guard<'a>: LockGuard<T>
    where
        Self: 'a;

    fn new(item: T) -> Result<Self, LockNewError>;

    fn lock<'a>(&'a self) -> Result<Self::Guard<'a>, SignalLockError>;
}

pub trait LockGuard<T>: Deref<Target = T> + DerefMut {}

#[cfg(target_os = "linux")]
mod imp {
    pub(crate) use crate::futex::{FutexLock as Lock, FutexLockGuard as LockGuard};
}

#[cfg(all(not(target_os = "linux"), target_family = "unix"))]
mod imp {
    pub(crate) use crate::pipe::{PipeLock as Lock, PipeLockGuard as LockGuard};
}

#[cfg(loom)]
mod fake_futex;

/// SignalLock is a mutex that is safe for use in signal handlers. It only uses atomics and
/// async-signal-safe primitives (e.g. futex or pipes). It does not allocate. It detects attempted
/// recursive locking by the same thread and returns an error instead of deadlocking (e.g. a signal
/// handler trying to take a lock already held by that thread).
///
/// On some platforms, this lock can perform similarly to std::sync::Mutex.
///
/// If you know your platform supports futex, you may prefer to directly use [futex::FutexLock],
/// which has a const-fn constructor and impls Default.
pub struct SignalLock<T> {
    // We wrap imp::Lock rather than export it because PipeLock::new and FutexLock::new don't have
    // the same signature. In particular, FutexLock::new is infallible (no Result) and const-fn.
    //
    // By wrapping, we ensure users can compile against a signature that doesn't change.
    inner: imp::Lock<T>,
}

unsafe impl<T: Send> Sync for SignalLock<T> {}

impl<T: Send> SignalLock<T> {
    #[inline]
    pub fn new(item: T) -> Result<SignalLock<T>, LockNewError> {
        Ok(SignalLock {
            inner: Lock::<T>::new(item)?,
        })
    }

    #[inline]
    pub fn lock<'a>(&'a self) -> Result<SignalLockGuard<'a, T>, SignalLockError> {
        self.inner.lock().map(|inner| SignalLockGuard { inner })
    }
}

impl<T: Send> Lock<T> for SignalLock<T> {
    type Guard<'a>
        = SignalLockGuard<'a, T>
    where
        Self: 'a;

    #[inline]
    fn new(item: T) -> Result<Self, LockNewError> {
        SignalLock::new(item)
    }

    #[inline]
    fn lock<'a>(&'a self) -> Result<Self::Guard<'a>, SignalLockError> {
        self.lock()
    }
}

impl<T> LockGuard<T> for SignalLockGuard<'_, T> {}

// Note that this impl is not async signal safe. We provide it for convenience.
impl<T: Send> Lock<T> for std::sync::Mutex<T> {
    type Guard<'a>
        = std::sync::MutexGuard<'a, T>
    where
        Self: 'a;

    #[inline]
    fn new(item: T) -> Result<Self, LockNewError> {
        Ok(std::sync::Mutex::new(item))
    }

    #[inline]
    fn lock<'a>(&'a self) -> Result<Self::Guard<'a>, SignalLockError> {
        // We don't support poisoning, so unfortunately, we unwrap here.
        Ok(self.lock().unwrap())
    }
}

impl<T> LockGuard<T> for std::sync::MutexGuard<'_, T> {}

/// This can make uncontended lock operations much faster by probing for a syscall-free alternative
/// for gettid.
///
/// After calling this, uncontended lock operations should be only 10% slower than
/// std::sync::Mutex. Otherwise on glibc, they can be 6-7 times slower.
///
/// Specifically, this looks for pthread_gettid_np, which was introduced in glibc 2.42 and allows
/// us to replace gettid() with pthread_gettid_np(pthread_self()).
///
/// On musl, gettid doesn't use syscalls and this function does nothing.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[ctor::ctor]
fn make_gettid_faster() {
    // Ideally we would use weak linkage, but that's not stabilized yet. Instead, we'll call
    // dlvsym.
    let ptr = unsafe {
        if cfg!(miri) {
            libc::dlsym(libc::RTLD_DEFAULT, c"pthread_gettid_np".as_ptr())
        } else {
            libc::dlvsym(
                libc::RTLD_DEFAULT,
                c"pthread_gettid_np".as_ptr(),
                c"GLIBC_2.42".as_ptr(),
            )
        }
    };
    if !ptr.is_null() {
        let tid = raw_gettid();
        let fnptr: FnPthreadGettidNp = unsafe { std::mem::transmute(ptr) };
        let new_tid = unsafe { fnptr(libc::pthread_self()) };
        // Double check it returns the same thing for at least this thread.
        if tid == new_tid {
            PTHREAD_GETTID_NP.store(fnptr, Ordering::SeqCst);
        }
    }
}

// It's hard to write generic code for fn pointers, so let's use a macro instead.
macro_rules! atomic_fn {
    ($name:ident, $fn:ty) => {
        #[repr(transparent)]
        struct $name(std::sync::atomic::AtomicPtr<()>);

        #[allow(unused)]
        impl $name {
            fn new(f: $fn) -> Self {
                // `f as usize` can't be used in a const fn.
                $name(std::sync::atomic::AtomicPtr::new(f as *const () as *mut ()))
            }

            const fn null() -> Self {
                $name(std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()))
            }

            fn load(&self, ordering: Ordering) -> Option<$fn> {
                let ptr = self.0.load(ordering) as *const ();
                if ptr.is_null() {
                    None
                } else {
                    // Safety: ptr is not null and was set with a valid function pointer in either
                    // new or store.
                    Some(unsafe { std::mem::transmute::<*const (), $fn>(ptr) })
                }
            }

            fn store(&self, f: $fn, ordering: Ordering) {
                self.0.store(f as *const () as *mut (), ordering);
            }
        }
    };
}

type FnPthreadGettidNp = unsafe extern "C" fn(libc::pthread_t) -> libc::pid_t;

atomic_fn!(AtomicPthreadGettidNp, FnPthreadGettidNp);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
static PTHREAD_GETTID_NP: AtomicPthreadGettidNp = AtomicPthreadGettidNp::null();

// Visible for tests
#[doc(hidden)]
pub type Tid = u32;

#[cfg(any(loom, miri))]
#[inline]
fn gettid_wrapper() -> Tid {
    // FYI tids can be reused after a thread dies. Our real code should handle that, but our
    // loom/miri fake signal code won't tolerate it, so make sure we never reuse tids here.

    // Thread id assignment doesn't need to be part of loom simulation.
    use std::sync::atomic::AtomicU32;
    static NEXT_TID: AtomicU32 = AtomicU32::new(1);

    #[cfg(loom)]
    use loom::thread_local;
    thread_local! {
        static TID: u32 = NEXT_TID.fetch_add(1, Ordering::Relaxed);
    }

    let tid = TID.with(|tid| *tid);
    if (tid & libc::FUTEX_TID_MASK) != tid {
        panic!("loom tid wraparound");
    }
    tid
}

#[cfg(all(not(any(loom, miri)), target_os = "linux", target_env = "gnu"))]
#[inline]
fn gettid_wrapper() -> Tid {
    if let Some(func) = PTHREAD_GETTID_NP.load(Ordering::Relaxed) {
        // unlike gettid, pthread_self and pthread_gettid_np don't use a syscall, so this is much
        // faster.
        let tpid = unsafe { libc::pthread_self() };
        return unsafe { func(tpid) } as Tid;
    }
    raw_gettid() as Tid
}

#[cfg(not(any(loom, miri)))]
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
#[inline]
fn gettid_wrapper() -> Tid {
    raw_gettid() as Tid
}

// Visible for tests
#[doc(hidden)]
#[inline]
pub fn gettid() -> Tid {
    gettid_wrapper()
}

#[cfg(target_os = "linux")]
#[inline]
fn raw_gettid() -> libc::c_int {
    unsafe { libc::gettid() }
}

#[cfg(target_os = "freebsd")]
#[inline]
fn raw_gettid() -> libc::c_int {
    unsafe { libc::pthread_getthreadid_np() }
}

#[cfg(target_os = "macos")]
#[inline]
fn raw_gettid() -> libc::c_int {
    let mut tid = 0;
    let res = unsafe { libc::pthread_threadid_np(libc::pthread_self(), &mut tid as *mut _) };
    if (res != 0) {
        abort_fmt!("pthread_threadid_np failed");
    }
    return tid;
}

#[derive(Debug)]
pub enum SignalLockError {
    /// This thread already holds the lock.
    Recursive,
}

pub struct SignalLockGuard<'a, T> {
    inner: imp::LockGuard<'a, T>,
}

impl<T> Deref for SignalLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.inner.deref()
    }
}

impl<T> DerefMut for SignalLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.inner.deref_mut()
    }
}

#[cfg(not(loom))]
struct StderrWriter {
    buf: std::io::Cursor<[u8; 1024]>,
    file: ManuallyDrop<std::fs::File>,
}

#[cfg(not(loom))]
impl StderrWriter {
    fn new() -> StderrWriter {
        use std::os::fd::FromRawFd;
        StderrWriter {
            buf: std::io::Cursor::new([0; _]),
            file: ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(libc::STDERR_FILENO) }),
        }
    }

    fn flush(&mut self) -> std::fmt::Result {
        use std::io::Write;
        let buf = &self.buf.get_ref()[..self.buf.position() as usize];
        let res = self.file.write_all(buf).map_err(|_| std::fmt::Error);
        self.buf.set_position(0);
        res
    }
}

#[cfg(not(loom))]
impl std::fmt::Write for StderrWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        use std::io::Write;
        // The buffer is not for performance (we are about to abort) but instead to reduce
        // interleaving in case other threads are also printing to stderr.
        let remaining = self.buf.get_ref().len() - self.buf.position() as usize;
        if s.len() < remaining {
            self.buf.write_all(s.as_bytes())
        } else {
            self.flush()?;
            if s.len() < self.buf.get_ref().len() {
                self.buf.write_all(s.as_bytes())
            } else {
                self.file.write_all(s.as_bytes())
            }
        }
        .map_err(|_| std::fmt::Error)
    }
}

#[cfg(not(loom))]
#[cold]
pub(crate) fn abort_fmt(args: std::fmt::Arguments) -> ! {
    use std::fmt::Write;
    let mut w = StderrWriter::new();
    let _ = std::fmt::write(&mut w, args)
        .map(|_| w.write_char('\n'))
        .map(|_| w.flush());
    std::process::abort();
}

/// Call this instead of panic!() when the code may run in a signal handler. The formatting code
/// for the arguments must not allocate or panic.
#[cfg(not(loom))]
macro_rules! abort {
    ($($args:tt)*) => (crate::abort_fmt(format_args!($($args)*)));
}

#[cfg(not(loom))]
use abort;

#[cfg(loom)]
use std::panic as abort;

#[cfg(any(loom, miri))]
mod fake_signal;

#[cfg(any(loom, miri))]
pub use fake_signal::*;

#[cfg(not(any(loom, miri)))]
#[inline(always)]
fn fake_signal_check_interrupt() {}
