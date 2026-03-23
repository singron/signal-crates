use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

#[cfg(not(loom))]
use std::sync::atomic::AtomicU32;

use crate::{SignalLockError, abort, fake_signal_check_interrupt, gettid};

#[cfg(loom)]
use crate::fake_futex::Futex;

#[cfg(not(loom))]
#[repr(transparent)]
pub(crate) struct Futex(AtomicU32);

// We can support any futex-like primitive as long as we can pack the tid and WAITERS bit. E.g. on
// linux, FUTEX_WAITERS uses a high bit, but on freebsd UMUTEX_CONTENDED uses a low bit (tids are >
// PID_MAX).

// TODO: freebsd _umtx_op, UMUTEX_CONTENDED
// TODO: darwin __ulock_wait/__ulock_wake (64-bit)

#[cfg(not(loom))]
impl Futex {
    const fn new(val: u32) -> Futex {
        Futex(AtomicU32::new(val))
    }

    pub(crate) fn atomic(&self) -> &AtomicU32 {
        &self.0
    }

    pub(crate) fn wait(&self, val: u32) {
        let res = unsafe { futex_wait_private(self.0.as_ptr(), val, std::ptr::null()) };
        if res == -1 {
            let err = errno::errno();
            if err.0 != libc::EAGAIN && err.0 != libc::EINTR && err.0 != libc::EWOULDBLOCK {
                abort!("futex wait failure: {:?}", err);
            }
            // Don't retry EINTR. Since we are awake, we may as well spin again.
        }
        // `man 2const FUTEX_WAIT` warns that res==0 can be a spurious wakeup.
    }

    pub(crate) fn wake_one(&self) {
        loop {
            let res = unsafe { futex_wake_private(self.0.as_ptr(), 1) };
            if res == -1 {
                let err = errno::errno();
                if err.0 == libc::EINTR {
                    // I don't think FUTEX_WAKE returns EINTR on linux, but retry just in case.
                    continue;
                }
                abort!("futex wake failure: {:?}", err);
            }
            break;
        }
    }
}

#[cfg(not(loom))]
unsafe fn futex_wait_private(
    uaddr: *const u32,
    val: u32,
    timeout: *const libc::timespec,
) -> libc::c_long {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            uaddr,
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            val,
            timeout,
        )
    }
}

#[cfg(not(loom))]
unsafe fn futex_wake_private(uaddr: *const u32, val: i32) -> libc::c_long {
    // Despite val being a u32 according to the man page for FUTEX_WAKE, it's used as an i32, and
    // the maximum value allowed is i32::MAX, not u32::MAX.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            uaddr,
            libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
            val as u32,
        )
    }
}

/// Remove const from fns under loom. The loom synchronization primitives don't have const fn
/// constructors, so our constructors can't be const either.
macro_rules! loom_strip_const {
    (pub const fn $name:ident($($args:tt)*) -> $ret:ty { $($body:tt)* }) => {
        #[cfg(not(loom))]
        pub const fn $name($($args)*) -> $ret { $($body)* }

        #[cfg(loom)]
        pub fn $name($($args)*) -> $ret { $($body)* }
    }
}

pub struct FutexLock<T> {
    item: UnsafeCell<T>,
    // This is the word we use for atomic operations and futex. If it's 0, the lock is available.
    // If it's held, then the lock holder's tid is within FUTEX_TID_MASK. If there are waiters that
    // require FUTEX_WAKE, then FUTEX_WAITERS is set. We use FUTEX_TID_MASK and FUTEX_WAITERS since
    // they are guaranteed to not overlap, but we don't use their priority inheritance
    // functionality.
    //
    // Otherwise, this is very similar to mutex2 in Futexes Are Tricky:
    // https://www.akkadia.org/drepper/futex.pdf
    futex: Futex,
}

impl<T> FutexLock<T> {
    loom_strip_const! {
        pub const fn new(item: T) -> FutexLock<T> {
            FutexLock { item: UnsafeCell::new(item), futex: Futex::new(0) }
        }
    }
}

impl<T> FutexLock<T> {
    #[inline]
    pub fn lock<'a>(&'a self) -> Result<FutexLockGuard<'a, T>, SignalLockError> {
        let tid = gettid();
        self.raw_lock(tid)?;
        Ok(FutexLockGuard {
            lock: self,
            tid,
            _unsend: PhantomData,
        })
    }

    #[inline]
    fn raw_lock(&self, tid: u32) -> Result<(), SignalLockError> {
        match self.futex.atomic().compare_exchange_weak(
            0,
            tid,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_prev) => Ok(()),
            Err(word) => self.raw_lock_contended(tid, word),
        }
    }

    #[cold]
    fn raw_lock_contended(&self, tid: u32, mut word: u32) -> Result<(), SignalLockError> {
        // TODO: Spin a little before waiting. Surprisingly, this is nearly as fast as
        // std::sync::Mutex in a variety of workloads, so I'm not too concerned.
        loop {
            // We must not wait if our tid holds the lock. This can happen if we are in a signal
            // handler that interrupted the thread while it was holding the lock. We check this on
            // every update just in case our own thread leaked the guard in a signal handler.
            if word & libc::FUTEX_TID_MASK == tid {
                return Err(SignalLockError::Recursive);
            }
            if word == 0 {
                match self.futex.atomic().compare_exchange_weak(
                    0,
                    tid | libc::FUTEX_WAITERS,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_prev) => return Ok(()),
                    Err(new_word) => word = new_word,
                }
                continue;
            }
            // A normal mutex might spin a few times, but we will go right to futex.
            if word & libc::FUTEX_WAITERS == 0 {
                match self.futex.atomic().compare_exchange_weak(
                    word,
                    word | libc::FUTEX_WAITERS,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    // holder tid not changed, so we can wait.
                    Ok(_prev) => word |= libc::FUTEX_WAITERS,
                    Err(new_word) => {
                        word = new_word;
                        continue;
                    }
                }
            }

            self.futex.wait(word);
            word = self.futex.atomic().load(Ordering::SeqCst);
        }
    }

    #[inline]
    fn raw_unlock(&self, tid: u32) {
        let prev = self.futex.atomic().swap(0, Ordering::Release);
        if cfg!(debug_assertions) {
            // SingalLockGuard isn't send, so this shouldn't happen.
            let holder_tid = prev & libc::FUTEX_TID_MASK;
            if holder_tid != tid {
                abort!(
                    "Unlocking mutex not held by this thread: tid={tid} holder_tid={holder_tid}\n"
                );
            }
        }
        if prev != tid {
            debug_assert_eq!(prev, tid | libc::FUTEX_WAITERS);
            self.raw_unlock_wake();
        }
    }

    #[cold]
    fn raw_unlock_wake(&self) {
        // I think this is one of the trickier parts of this lock. At this point, we can be
        // interrupted and our signal handler can try to take the lock. Since our thread isn't
        // holding it, it may wait on the futex. However, this thread could be responsible for
        // waking a thread, and it's not obvious whether it could become responsible for waking
        // itself!
        //
        // The TLA spec and loom tests gives me a lot of confidence that this isn't possible.
        // Informally, in order for this thread to wait on the futex in the signal handler, there
        // must be some other thread holding the lock, and that thread is responsible for waking
        // this thread (or waking a different thread that takes the lock and wakes this thread, and
        // so on).
        fake_signal_check_interrupt();
        self.futex.wake_one();
    }
}

impl<T: Default> Default for FutexLock<T> {
    fn default() -> Self {
        FutexLock::new(Default::default())
    }
}

pub struct FutexLockGuard<'a, T> {
    lock: &'a FutexLock<T>,
    /// Cache the tid so we don't call gettid in unlock. See [make_faster]
    tid: u32,

    /// Force !Send. SignalLock uses the TID, so it must be unlocked on the same thread it was
    /// locked on.
    _unsend: PhantomData<std::sync::MutexGuard<'a, ()>>,
}

impl<T> Drop for FutexLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.raw_unlock(self.tid);
    }
}

impl<T> Deref for FutexLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // Safety: We exclusively hold the lock.
        unsafe { &*self.lock.item.get() }
    }
}

impl<T> DerefMut for FutexLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: We exclusively hold the lock.
        unsafe { &mut *self.lock.item.get() }
    }
}

unsafe impl<T: Send> Sync for FutexLock<T> {}

impl<T: Send> crate::Lock<T> for FutexLock<T> {
    type Guard<'a>
        = FutexLockGuard<'a, T>
    where
        Self: 'a;

    fn new(item: T) -> Result<Self, crate::LockNewError> {
        Ok(FutexLock::new(item))
    }

    fn lock<'a>(&'a self) -> Result<Self::Guard<'a>, SignalLockError> {
        self.lock()
    }
}

impl<T> crate::LockGuard<T> for FutexLockGuard<'_, T> {}
