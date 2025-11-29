use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use crossbeam_channel::bounded;
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// 测试1: Rayon par_iter 在不同线程数下的调度开销
fn bench_rayon_thread_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("rayon_scaling");

    let items: Vec<usize> = (0..10000).collect();

    // 顺序执行作为基准
    group.bench_function("sequential_10k", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for x in &items {
                sum = sum.wrapping_add(black_box(*x));
            }
            sum
        })
    });

    // 不同线程数的 Rayon
    for threads in [1, 2, 4, 8, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();

        group.bench_with_input(
            BenchmarkId::new("rayon_par_iter", threads),
            &threads,
            |b, _| {
                b.iter(|| {
                    pool.install(|| {
                        items.par_iter().map(|x| black_box(*x)).sum::<usize>()
                    })
                })
            },
        );
    }

    group.finish();
}

/// 测试2: Channel 在不同生产者/消费者数量下的开销
fn bench_channel_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_contention");
    group.measurement_time(Duration::from_secs(3));

    let items_per_test = 10000;

    // 单生产者单消费者
    group.bench_function("spsc", |b| {
        let (tx, rx) = bounded::<usize>(1000);
        b.iter(|| {
            let tx = tx.clone();
            let rx = rx.clone();

            let producer = thread::spawn(move || {
                for i in 0..items_per_test {
                    tx.send(i).unwrap();
                }
            });

            let consumer = thread::spawn(move || {
                for _ in 0..items_per_test {
                    black_box(rx.recv().unwrap());
                }
            });

            producer.join().unwrap();
            consumer.join().unwrap();
        })
    });

    // 多生产者单消费者 (模拟 scanner -> queue)
    group.bench_function("mpsc_4_producers", |b| {
        let (tx, rx) = bounded::<usize>(1000);
        b.iter(|| {
            let producers: Vec<_> = (0..4).map(|_| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for i in 0..items_per_test/4 {
                        tx.send(i).unwrap();
                    }
                })
            }).collect();

            let rx = rx.clone();
            let consumer = thread::spawn(move || {
                for _ in 0..items_per_test {
                    black_box(rx.recv().unwrap());
                }
            });

            for p in producers { p.join().unwrap(); }
            consumer.join().unwrap();
        })
    });

    // 多生产者多消费者 (模拟完整的 scanner pool -> deleter pool)
    group.bench_function("mpmc_4x4", |b| {
        let (tx, rx) = bounded::<usize>(1000);
        b.iter(|| {
            let producers: Vec<_> = (0..4).map(|_| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for i in 0..items_per_test/4 {
                        tx.send(i).unwrap();
                    }
                })
            }).collect();

            let consumers: Vec<_> = (0..4).map(|_| {
                let rx = rx.clone();
                thread::spawn(move || {
                    loop {
                        match rx.recv_timeout(Duration::from_millis(10)) {
                            Ok(v) => { black_box(v); }
                            Err(_) => break,
                        }
                    }
                })
            }).collect();

            for p in producers { p.join().unwrap(); }
            for c in consumers { c.join().unwrap(); }
        })
    });

    group.finish();
}

/// 测试3: 模拟真实的 scan + delete 流水线
fn bench_pipeline_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");
    group.measurement_time(Duration::from_secs(5));

    let items = 10000;

    // 基准: 顺序处理 (无线程开销)
    group.bench_function("sequential", |b| {
        b.iter(|| {
            let mut results = Vec::with_capacity(items);
            for i in 0..items {
                // 模拟 scan
                let job = black_box(i);
                // 模拟 delete
                results.push(job * 2);
            }
            results
        })
    });

    // 单池: Rayon 直接处理
    group.bench_function("single_pool_rayon", |b| {
        b.iter(|| {
            (0..items).into_par_iter()
                .map(|i| {
                    let job = black_box(i);
                    job * 2
                })
                .collect::<Vec<_>>()
        })
    });

    // 两池: Scanner pool -> Queue -> Deleter pool (当前架构)
    group.bench_function("two_pool_with_queue", |b| {
        let (tx, rx) = bounded::<usize>(10000);

        b.iter(|| {
            let tx = tx.clone();
            let rx = rx.clone();

            // Scanner pool
            let scanner = thread::spawn(move || {
                (0..items).into_par_iter().for_each(|i| {
                    tx.send(black_box(i)).unwrap();
                });
            });

            // Deleter pool
            let deleter = thread::spawn(move || {
                let mut results = Vec::with_capacity(items);
                for _ in 0..items {
                    let job = rx.recv().unwrap();
                    results.push(black_box(job) * 2);
                }
                results
            });

            scanner.join().unwrap();
            deleter.join().unwrap()
        })
    });

    group.finish();
}

/// 测试4: OS 线程上下文切换开销
fn bench_os_context_switch(c: &mut Criterion) {
    let mut group = c.benchmark_group("os_context_switch");

    let num_cpus = num_cpus::get();
    let iterations = 100000;

    // 线程数 = CPU 核心数 (理想情况)
    group.bench_function("threads_eq_cpus", |b| {
        b.iter(|| {
            let counter = Arc::new(AtomicUsize::new(0));
            let per_thread = iterations / num_cpus;

            let handles: Vec<_> = (0..num_cpus).map(|_| {
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..per_thread {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            }).collect();

            for h in handles { h.join().unwrap(); }
            counter.load(Ordering::Relaxed)
        })
    });

    // 线程数 = 2x CPU (轻度过度订阅)
    group.bench_function("threads_2x_cpus", |b| {
        b.iter(|| {
            let counter = Arc::new(AtomicUsize::new(0));
            let threads = num_cpus * 2;
            let per_thread = iterations / threads;

            let handles: Vec<_> = (0..threads).map(|_| {
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..per_thread {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            }).collect();

            for h in handles { h.join().unwrap(); }
            counter.load(Ordering::Relaxed)
        })
    });

    // 线程数 = 4x CPU (重度过度订阅)
    group.bench_function("threads_4x_cpus", |b| {
        b.iter(|| {
            let counter = Arc::new(AtomicUsize::new(0));
            let threads = num_cpus * 4;
            let per_thread = iterations / threads;

            let handles: Vec<_> = (0..threads).map(|_| {
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..per_thread {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            }).collect();

            for h in handles { h.join().unwrap(); }
            counter.load(Ordering::Relaxed)
        })
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(100);
    targets =
        bench_rayon_thread_scaling,
        bench_channel_contention,
        bench_pipeline_overhead,
        bench_os_context_switch
}

criterion_main!(benches);
