use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

#[cfg(not(loom))]
use std::sync::atomic::{AtomicU32, fence};

#[cfg(loom)]
use loom::sync::atomic::{AtomicU32, fence};

#[cfg(loom)]
use loom::sync::{Condvar, Mutex};

use crate::{SignalLockError, Tid, abort, fake_signal_check_interrupt, gettid};

#[cfg(not(loom))]
struct Fd(libc::c_int);

#[cfg(not(loom))]
impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

#[derive(Debug)]
pub enum PipeNewError {
    PipeFailed(std::io::Error),
    FcntlFailed(std::io::Error),
}

/// Return (reader, writer). Writer is non-blocking.
#[cfg(not(loom))]
fn pipe() -> Result<(Fd, Fd), PipeNewError> {
    let mut fds = [0; 2];
    let err = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if err != 0 {
        return Err(PipeNewError::PipeFailed(std::io::Error::last_os_error()));
    }
    let fds = (Fd(fds[0]), Fd(fds[1]));

    let err = unsafe { libc::fcntl(fds.1.0, libc::F_SETFL, libc::O_NONBLOCK) };
    if err != 0 {
        return Err(PipeNewError::FcntlFailed(std::io::Error::last_os_error()));
    }

    Ok(fds)
}

#[cfg(loom)]
struct LoomPipe {
    mutex: Mutex<u32>,
    condvar: Condvar,
}

#[cfg(loom)]
impl LoomPipe {
    fn new() -> LoomPipe {
        LoomPipe {
            mutex: Mutex::new(0),
            condvar: Condvar::new(),
        }
    }

    fn read(&self) -> libc::ssize_t {
        let mut g = self.mutex.lock().unwrap();
        loop {
            if *g > 0 {
                *g -= 1;
                return 1;
            }
            g = self.condvar.wait(g).unwrap();
        }
    }

    fn write(&self) -> libc::ssize_t {
        let mut g = self.mutex.lock().unwrap();
        let is_write = (*g as usize) < 2;
        // Normally, we would stop writing at the pipe capacity, which is probably several KiB, but
        // we will pretend the capacity is lower so it could be possible to hit with loom.
        if is_write {
            *g += 1;
        }
        drop(g);
        if is_write {
            self.condvar.notify_all();
            1
        } else {
            errno::set_errno(errno::Errno(libc::EWOULDBLOCK));
            -1
        }
    }
}

/// A signal safe lock based on atomics, pipe(2), read(2), and write(2).
///
/// See [crate::SignalLock], which provides more documentation and will select the best
/// implementation available for your platform.
///
/// While this is the most compatible implementation, the futex one is generally faster, doesn't
/// use any file descriptors, and has a const-fn infallible constructor.
pub struct PipeLock<T> {
    holder_tid: AtomicU32,
    waiters: AtomicU32,
    #[cfg(not(loom))]
    reader: Fd,
    #[cfg(not(loom))]
    writer: Fd,
    #[cfg(loom)]
    pipe: LoomPipe,
    item: UnsafeCell<T>,
}

// [TRICKY MEMORY MODEL STUFF]
//
// Because we lack futex, we instead use 2 atomic variables (holder_tid, waiters). holder_tid
// protects the critical section with Acquire/Release semantics by itself. The tricky part is that
// we must ensure that if a locker waits on the pipe, an unlocker will always see and wake them,
// which requires specific relationships between the two variables. Lockers will increment waiters
// and then check holder_tid != 0. Unlockers will set holder_tid=0 and then check
// waiters>0. If you assume sequential consistencey, then that works (and is why the TLA spec
// passes).
//
// Unfortunately, loom doesn't support SeqCst on atomic accesses (they are weakened to AcqRel).
// Instead, we will provide the needed ordering with fences. As a bonus, this also uses fewer
// memory barrier instructions on weak memory platforms. Below are simplified versions of the
// relevant portions of the code:
//
//  Locker:                     Unlocker:
//
//  A:  waiters += 1;           X:  holder_tid=tid; // entering critical section
//  F1: fence(SeqCst);          Y:  holder_tid=0; (Release) // leaving critical section
//  B:  if holder_tid != 0 {    F2: fence(SeqCst);
//        read(); // wait       Z:  if waiters > 0; {
//      }                             write(); // wake
//                                  }
//
// To prevent the Locker from waiting without being woken, we need (B reads from X) to imply (Z
// reads from A). Informally, the fences are ordered in relation to each other and don't permit
// operations being reordered past them, which seems like it should fix the problem.
//
// But the C++20 memory model is complicated and permits many strange behaviors, so let's be
// precise. I'm not going to explain all these terms or rules, but there are definitions at
// https://en.cppreference.com/w/cpp/atomic/memory_order.html
//
// Proof by contradiction: assume the Locker waits without being woken. I.e.
// * B reads from X.
// * Z reads from a value before A.
//
// Then:
//
// * The sequenced-before relations (i.e. program order) are
//   * A -> F1 -> B
//   * X -> Y -> F2 -> Z
// * Operations that are sequenced-before are also happens-before.
// * Z is coherence-ordered-before A since it reads a value before A.
// * Since F2 happens-before Z, and A happens-before F1, and Z is coherence-ordered-before A, then
//   F2 precedes F1 in the sequential consistency total order S.
//
// * B is coherence-ordered-before Y since it reads from X which is before Y.
// * Since F1 happens-before B, and Y happens-before F2, and B is coherence-ordered-before Y, then
// F1 precedes F2 in the total order S.
//
// * Contradiction: F1 precedes F2, but F2 also precedes F1.
// * Therefore, the Locker can't wait without being woken.

impl<T> PipeLock<T> {
    pub fn new(item: T) -> Result<Self, PipeNewError> {
        #[cfg(not(loom))]
        let (reader, writer) = pipe()?;

        Ok(PipeLock {
            holder_tid: AtomicU32::new(0),
            #[cfg(not(loom))]
            reader,
            #[cfg(not(loom))]
            writer,
            #[cfg(loom)]
            pipe: LoomPipe::new(),
            waiters: AtomicU32::new(0),
            item: UnsafeCell::new(item),
        })
    }

    #[inline]
    pub fn lock<'a>(&'a self) -> Result<PipeLockGuard<'a, T>, SignalLockError> {
        let tid = gettid();
        match self
            .holder_tid
            .compare_exchange_weak(0, tid, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Ok(PipeLockGuard {
                lock: self,
                tid,
                contended: false,
                _unsend: PhantomData,
            }),
            Err(holder_tid) => self.lock_contended(tid, holder_tid),
        }
    }

    #[cold]
    fn lock_contended<'a>(
        &'a self,
        tid: Tid,
        mut holder_tid: Tid,
    ) -> Result<PipeLockGuard<'a, T>, SignalLockError> {
        if holder_tid == tid {
            return Err(SignalLockError::Recursive);
        }

        self.waiters.fetch_add(1, Ordering::Relaxed);
        fake_signal_check_interrupt();
        // See [TRICKY MEMORY MODEL STUFF]
        fence(Ordering::SeqCst);

        // TODO: Spin a little before waiting.

        loop {
            match self.holder_tid.compare_exchange_weak(
                0,
                tid,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.waiters.fetch_sub(1, Ordering::Relaxed);
                    return Ok(PipeLockGuard {
                        lock: self,
                        tid,
                        contended: true,
                        _unsend: PhantomData,
                    });
                }
                Err(new_holder_tid) => holder_tid = new_holder_tid,
            }
            if holder_tid == tid {
                self.waiters.fetch_sub(1, Ordering::Relaxed);
                return Err(SignalLockError::Recursive);
            }
            if holder_tid == 0 {
                continue;
            }

            let res = self.read();
            if res < 0 {
                let err = errno::errno();
                if err.0 != libc::EINTR {
                    self.waiters.fetch_sub(1, Ordering::Relaxed);
                    // Most standard errors should be impossible (EPIPE, EAGAIN/EWOULDBLOCK, etc.).
                    // EIO and ENOMEM can always be thrown, but I'm not sure it's possible in
                    // practice, and it's probably not recoverable anyway.
                    abort!("PipeLock read failed: {:?}", err);
                }
            }
            if res == 0 {
                abort!("PipeLock writer fd was closed");
            }
        }
    }

    #[inline]
    #[cfg(not(loom))]
    fn read(&self) -> libc::ssize_t {
        let mut buf = [0; 1];
        unsafe { libc::read(self.reader.0, buf.as_mut_ptr() as *mut _, buf.len()) }
    }

    #[cfg(loom)]
    fn read(&self) -> libc::ssize_t {
        self.pipe.read()
    }

    #[inline]
    fn unlock(&self, tid: Tid, contended: bool) {
        let holder_tid = self.holder_tid.swap(0, Ordering::Release);
        if cfg!(debug_assertions) && tid != holder_tid {
            // The guard is !Send, so this should be impossible.
            abort!("Unlocked mutex not held by this thread: tid={tid} holder_tid={holder_tid}\n");
        }

        fake_signal_check_interrupt();

        // See [TRICKY MEMORY MODEL STUFF]
        if contended || {
            fence(Ordering::SeqCst);
            self.waiters.load(Ordering::Relaxed) > 0
        } {
            self.unlock_contended();
        }
    }

    #[cold]
    fn unlock_contended(&self) {
        loop {
            let res = self.write();
            if res < 0 {
                let err = errno::errno();
                if err.0 == libc::EINTR {
                    continue;
                }
                if !(err.0 == libc::EAGAIN || err.0 == libc::EWOULDBLOCK) {
                    abort!("PipeLock write failed: {:?}", err);
                }
            }
            break;
        }
    }

    #[cfg(not(loom))]
    fn write(&self) -> libc::ssize_t {
        let buf = [0; 1];
        unsafe { libc::write(self.writer.0, buf.as_ptr() as *const _, buf.len()) }
    }

    #[cfg(loom)]
    fn write(&self) -> libc::ssize_t {
        self.pipe.write()
    }
}

pub struct PipeLockGuard<'a, T> {
    lock: &'a PipeLock<T>,
    /// Cache the tid so we don't call gettid in unlock.
    tid: u32,
    /// Was the lock contended when we locked it.
    contended: bool,

    /// Force !Send. SignalLock uses the TID, so it must be unlocked on the same thread it was
    /// locked on.
    _unsend: PhantomData<std::sync::MutexGuard<'a, ()>>,
}

impl<T> Drop for PipeLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.unlock(self.tid, self.contended);
    }
}

impl<T> Deref for PipeLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // Safety: We exclusively hold the lock.
        unsafe { &*self.lock.item.get() }
    }
}

impl<T> DerefMut for PipeLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: We exclusively hold the lock.
        unsafe { &mut *self.lock.item.get() }
    }
}

unsafe impl<T: Send> Sync for PipeLock<T> {}

impl<T: Send> crate::Lock<T> for PipeLock<T> {
    type Guard<'a>
        = PipeLockGuard<'a, T>
    where
        Self: 'a;

    fn new(item: T) -> Result<Self, crate::LockNewError> {
        PipeLock::new(item)
    }

    fn lock<'a>(&'a self) -> Result<Self::Guard<'a>, SignalLockError> {
        self.lock()
    }
}

impl<T> crate::LockGuard<T> for PipeLockGuard<'_, T> {}
