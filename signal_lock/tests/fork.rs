#[cfg(any(loom, miri))]
fn main() {
    println!("test fork ignored");
}

#[cfg(not(any(loom, miri)))]
fn main() {
    // The guard caches the locker's tid, so it can unlock after a fork even though the tid would
    // have changed.
    let lock1 = signal_lock::SignalLock::new(()).unwrap();
    let lock2 = signal_lock::pipe::PipeLock::new(()).unwrap();
    let g1 = lock1.lock().unwrap();
    let g2 = lock2.lock().unwrap();
    let pid = unsafe { libc::fork() };
    drop(g1);
    drop(g2);
    if pid == 0 {
        // Use exit instead of std::process::exit to avoid running cleanup from the parent.
        unsafe { libc::exit(0) };
    }
    assert_ne!(-1, pid, "fork failed");
    let mut status = 0;
    let res = unsafe { libc::waitpid(pid, &mut status as *mut _, 0) };
    assert_eq!(pid, res, "waitpid failed");
    if !libc::WIFEXITED(status) {
        let sig = libc::WTERMSIG(status);
        panic!("Child failed with {sig}");
    }
    assert_eq!(0, libc::WEXITSTATUS(status));
    println!("test fork passed");
}
