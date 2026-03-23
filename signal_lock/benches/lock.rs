use criterion::{Bencher, Criterion, criterion_group, criterion_main};
#[cfg(target_os = "linux")]
use signal_lock::futex::FutexLock;
use signal_lock::{Lock, pipe::PipeLock};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Barrier, Mutex};
use std::time::Instant;

fn bench_gettid(c: &mut Criterion) {
    // Because we have a user-space fast-path, we are very sensitive to gettid performance.
    #[cfg(target_os = "linux")]
    c.bench_function("raw_gettid_1000", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                black_box(unsafe { libc::gettid() });
            }
        })
    });
    c.bench_function("gettid_1000", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                black_box(signal_lock::gettid());
            }
        })
    });
}
criterion_group!(benches_gettid, bench_gettid);

fn contend_lock<L: Lock<u64>>(lock: &L, n: u64) -> u64 {
    for _ in 0..(n - 1) {
        *lock.lock().unwrap() += 1;
    }
    return *lock.lock().unwrap();
}

fn spin_a_bit(a: &AtomicBool) -> u64 {
    for i in 0..100 {
        if a.load(Ordering::Relaxed) {
            return i;
        }
    }
    100
}

fn contend_lock_spin<L: Lock<u64>>(lock: &L, n: u64) -> u64 {
    for _ in 0..(n - 1) {
        {
            *lock.lock().unwrap() += 1;
        }
        // Try to waste time outside the lock so another thread can grab it.
        black_box(spin_a_bit(black_box(&AtomicBool::new(false))));
    }
    return *lock.lock().unwrap();
}

fn contend_lock_with_threads<L: Lock<u64>, F: Fn(&L, u64) -> u64 + Send + Sync>(
    b: &mut Bencher,
    threads: u32,
    rounds: u64,
    looper: F,
) {
    let lock = L::new(0u64).unwrap();
    let barrier = Barrier::new(threads as usize + 1);
    let stop = AtomicBool::new(false);
    let sum = &AtomicU64::new(0);
    std::thread::scope(|scope| {
        let mut thread_handles = Vec::new();
        for _ in 0..threads {
            thread_handles.push(scope.spawn(|| {
                loop {
                    barrier.wait();
                    if stop.load(Ordering::SeqCst) {
                        return sum;
                    }
                    sum.fetch_add(looper(&lock, rounds), Ordering::SeqCst);
                    barrier.wait();
                }
            }));
        }
        b.iter(|| {
            barrier.wait();
            barrier.wait();
            black_box(sum.load(Ordering::SeqCst))
        });
        stop.store(true, Ordering::SeqCst);
        barrier.wait();
    })
}

fn bench_contend(c: &mut Criterion) {
    let threads_values = [1, 2, 4, 8, 16, 256];

    #[cfg(target_os = "linux")]
    for threads in threads_values {
        c.bench_function(&format!("futex_contend_{threads}_1000"), |b| {
            contend_lock_with_threads::<FutexLock<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock,
            )
        });
    }
    for threads in threads_values {
        c.bench_function(&format!("pipe_contend_{threads}_1000"), |b| {
            contend_lock_with_threads::<PipeLock<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock,
            )
        });
    }
    for threads in threads_values {
        c.bench_function(&format!("mutex_contend_{threads}_1000"), |b| {
            contend_lock_with_threads::<Mutex<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock,
            )
        });
    }

    #[cfg(target_os = "linux")]
    for threads in threads_values {
        c.bench_function(&format!("futex_contend_spin_{threads}_1000"), |b| {
            contend_lock_with_threads::<FutexLock<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock_spin,
            )
        });
    }
    for threads in threads_values {
        c.bench_function(&format!("pipe_contend_spin_{threads}_1000"), |b| {
            contend_lock_with_threads::<PipeLock<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock_spin,
            )
        });
    }
    for threads in threads_values {
        c.bench_function(&format!("mutex_contend_spin_{threads}_1000"), |b| {
            contend_lock_with_threads::<Mutex<u64>, _>(
                b,
                black_box(threads),
                black_box(1000),
                contend_lock_spin,
            )
        });
    }
}
criterion_group!(benches_contend, bench_contend);

fn ping_pong<L: Lock<()>>(b: &mut Bencher) {
    b.iter_custom(|iter| {
        let lock = L::new(()).unwrap();
        let count = AtomicU64::new(black_box(0));
        let barrier = Barrier::new(2);
        let rounds = black_box(100);

        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                barrier.wait();
                for _ in 0..iter {
                    for _ in 0..rounds {
                        while count.load(Ordering::Relaxed) & 1 == 0 {
                            std::hint::spin_loop();
                        }
                        {
                            let _g = lock.lock().unwrap();
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });

            barrier.wait();
            let start = Instant::now();
            for _ in 0..iter {
                for _ in 0..rounds {
                    {
                        let _g = lock.lock().unwrap();
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                    while count.load(Ordering::Relaxed) & 1 == 1 {
                        std::hint::spin_loop();
                    }
                }
            }

            let elapsed = start.elapsed();
            drop(handle);
            black_box(count.load(Ordering::Relaxed));
            elapsed
        })
    })
}

fn bench_ping_pong(c: &mut Criterion) {
    #[cfg(target_os = "linux")]
    c.bench_function("futex_ping_pong", ping_pong::<FutexLock<()>>);
    c.bench_function("pipe_ping_pong", ping_pong::<PipeLock<()>>);
    c.bench_function("mutex_ping_pong", ping_pong::<Mutex<()>>);
}
criterion_group!(benches_ping_pong, bench_ping_pong);

criterion_main!(benches_gettid, benches_contend, benches_ping_pong);
