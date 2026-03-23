use std::sync::atomic::{Ordering, AtomicPtr, AtomicU32};
use std::thread::Thread;

use crate::{Tid, SignalLockError};

struct Parker {
    #[cfg(target_os="linux")]
    futex: crate::futex::Futex,
}

#[cfg(target_os="linux")]
impl Parker {
    fn park(&self) {
        loop {
            if self.futex.atomic().swap(0, Ordering::Acquire) != 0 {
                break
            }
            self.futex.wait(0);
        }
    }

    fn unpark(&self, _tid: Tid) {
        self.futex.atomic().store(1, Ordering::Release);
        self.futex.wake_one();
    }
}

#[cfg(target_os="illumos")]
extern "C" {
    fn lwp_park(
}

#[cfg(target_os="illumos")]
impl Parker {
    fn park(&self) {
        libc::_lwp_park(std::ptr::null());
    }

    fn unpark(&self, tid: Tid) {
        self.futex.wake_one();
    }
}

struct ParkLock {
    tid: AtomicU32,
    head: AtomicPtr<WaitEntry>,
    tail: AtomicPtr<WaitEntry>,
}

/// WaitEntry is stack allocated on a waiting thread. The waiting thread cannot deallocate the
/// entry until it is woken and tid is 0. If tid is not 0 after waking, then the wake was spurious.
struct WaitEntry {
    prev: AtomicPtr<WaitEntry>,
    next: AtomicPtr<WaitEntry>,
    tid: AtomicU32,
    park: Parker,
}

impl ParkLock {
    fn raw_lock(&self) -> Result<(), SignalLockError> {
        let tid = crate::gettid();
        match self.tid.compare_exchange_weak(0, tid, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => Ok(()),
            Err(holder_tid) => self.raw_lock_contended(tid, holder_tid),
        }
    }

    #[cold]
    fn raw_lock_contended(&self, tid: Tid, holder_tid: Tid) -> Result<(), SignalLockError> {
        if tid == holder_tid {
            return Err(SignalLockError::Recursive);
        }

        let head = self.head.load(Ordering::Relaxed);
        loop {
            let mut entry = WaitEntry {
                tid,
                prev: AtomicPtr::new(std::ptr::null_mut()),
                next: AtomicPtr::new(head),
            };
            let entry_ptr = &mut entry as *mut _ ;
            // ABA is possible here, but we don't care since we still have the correct head
            // pointer.
            match self.head.compare_exchange_weak(head, entry_ptr, Ordering::SeqCst, Ordering::Relaxed) {
            }

            break;
        }

        Ok(())
    }
}
