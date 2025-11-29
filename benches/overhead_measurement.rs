use criterion::{black_box, criterion_group, criterion_main, Criterion};
use crossbeam_channel::{bounded, unbounded};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::path::Path;
use std::time::Duration;

fn measure_atomic_ops(c: &mut Criterion) {
    let counter = AtomicUsize::new(0);

    c.bench_function("atomic_fetch_add_relaxed", |b| {
        b.iter(|| {
            counter.fetch_add(1, Ordering::Relaxed)
        })
    });

    c.bench_function("atomic_load_relaxed", |b| {
        b.iter(|| {
            counter.load(Ordering::Relaxed)
        })
    });
}

fn measure_arc_allocation(c: &mut Criterion) {
    let path = Path::new("/tmp/test/some/path/file.txt");

    c.bench_function("arc_from_path", |b| {
        b.iter(|| {
            let arc: Arc<Path> = Arc::from(black_box(path));
            arc
        })
    });

    // 对比：PathBuf clone
    let pathbuf = path.to_path_buf();
    c.bench_function("pathbuf_clone", |b| {
        b.iter(|| {
            black_box(&pathbuf).clone()
        })
    });
}

fn measure_channel_ops(c: &mut Criterion) {
    // Bounded channel (类似当前实现)
    let (tx, rx) = bounded::<usize>(10000);

    c.bench_function("channel_bounded_send", |b| {
        b.iter(|| {
            tx.send(black_box(42)).unwrap();
            rx.recv().unwrap()
        })
    });

    // try_send (非阻塞)
    let (tx2, rx2) = bounded::<usize>(10000);
    c.bench_function("channel_try_send", |b| {
        b.iter(|| {
            let _ = tx2.try_send(black_box(42));
            let _ = rx2.try_recv();
        })
    });

    // Unbounded channel
    let (tx3, rx3) = unbounded::<usize>();
    c.bench_function("channel_unbounded_send", |b| {
        b.iter(|| {
            tx3.send(black_box(42)).unwrap();
            rx3.recv().unwrap()
        })
    });
}

fn measure_syscalls(c: &mut Criterion) {
    // 创建测试文件
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("bench_test_file");
    std::fs::write(&test_file, "test").ok();

    c.bench_function("syscall_metadata (stat)", |b| {
        b.iter(|| {
            std::fs::metadata(black_box(&test_file))
        })
    });

    c.bench_function("syscall_symlink_metadata (lstat)", |b| {
        b.iter(|| {
            std::fs::symlink_metadata(black_box(&test_file))
        })
    });

    // 清理
    std::fs::remove_file(&test_file).ok();
}

fn measure_rayon_overhead(c: &mut Criterion) {
    use rayon::prelude::*;

    // 测量 rayon par_iter 在小任务上的开销
    let items: Vec<usize> = (0..1000).collect();

    c.bench_function("rayon_par_iter_1000_noop", |b| {
        b.iter(|| {
            items.par_iter().for_each(|x| {
                black_box(x);
            })
        })
    });

    // 对比：普通迭代
    c.bench_function("sequential_iter_1000_noop", |b| {
        b.iter(|| {
            items.iter().for_each(|x| {
                black_box(x);
            })
        })
    });

    // par_bridge 开销 (用于 read_dir)
    let items2: Vec<usize> = (0..100).collect();
    c.bench_function("rayon_par_bridge_100_noop", |b| {
        b.iter(|| {
            items2.iter().par_bridge().for_each(|x| {
                black_box(x);
            })
        })
    });
}

fn measure_combined_file_overhead(c: &mut Criterion) {
    // 模拟完整的文件处理流程
    let (tx, rx) = bounded::<Arc<Path>>(10000);
    let path = Path::new("/tmp/test/file.txt");
    let counter = AtomicUsize::new(0);

    c.bench_function("full_file_overhead_simulation", |b| {
        b.iter(|| {
            // Scanner side
            counter.fetch_add(1, Ordering::Relaxed);  // inc_scanned
            let arc: Arc<Path> = Arc::from(black_box(path));
            tx.send(arc).unwrap();

            // Deleter side
            let _path = rx.recv().unwrap();
            counter.fetch_add(1, Ordering::Relaxed);  // inc_deleted
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(3))
        .sample_size(1000);
    targets =
        measure_atomic_ops,
        measure_arc_allocation,
        measure_channel_ops,
        measure_syscalls,
        measure_rayon_overhead,
        measure_combined_file_overhead
}

criterion_main!(benches);
