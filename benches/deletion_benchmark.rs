use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Create a directory structure with N files
fn create_flat_structure(base: &std::path::Path, num_files: usize) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for i in 0..num_files {
        let file_path = base.join(format!("file_{}.txt", i));
        File::create(&file_path).unwrap();
        paths.push(file_path);
    }
    paths
}

/// Create a nested directory structure with depth D and breadth B
fn create_nested_structure(
    base: &std::path::Path,
    depth: usize,
    breadth: usize,
) -> (Vec<PathBuf>, usize) {
    let mut paths = Vec::new();
    let mut total_items = 0;

    fn recurse(
        path: &std::path::Path,
        depth: usize,
        breadth: usize,
        paths: &mut Vec<PathBuf>,
        total: &mut usize,
    ) {
        if depth == 0 {
            return;
        }

        // Create files in this directory
        for i in 0..breadth {
            let file = path.join(format!("file_{}.txt", i));
            File::create(&file).unwrap();
            paths.push(file);
            *total += 1;
        }

        // Create subdirectories
        for i in 0..breadth {
            let dir = path.join(format!("dir_{}", i));
            fs::create_dir(&dir).unwrap();
            *total += 1;
            recurse(&dir, depth - 1, breadth, paths, total);
        }
    }

    recurse(base, depth, breadth, &mut paths, &mut total_items);
    (paths, total_items)
}

/// Benchmark fast-rm with different configurations
fn bench_fast_rm_flat(c: &mut Criterion) {
    let mut group = c.benchmark_group("fast_rm_flat");

    for size in [100, 1000, 5000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_with_setup(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    create_flat_structure(temp_dir.path(), size);
                    temp_dir
                },
                |temp_dir| {
                    let path = temp_dir.path().to_path_buf();
                    let output = Command::new("cargo")
                        .args(&["run", "--release", "--"])
                        .arg(&path)
                        .output()
                        .expect("Failed to run fast-rm");

                    black_box(output);
                },
            );
        });
    }

    group.finish();
}

/// Benchmark nested structure deletion
fn bench_fast_rm_nested(c: &mut Criterion) {
    let mut group = c.benchmark_group("fast_rm_nested");

    // (depth, breadth) configurations
    for &(depth, breadth) in &[(3, 5), (4, 4), (5, 3)] {
        let temp_dir = TempDir::new().unwrap();
        let (_, total_items) = create_nested_structure(temp_dir.path(), depth, breadth);

        group.throughput(Throughput::Elements(total_items as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("d{}b{}", depth, breadth)),
            &(depth, breadth),
            |b, &(d, br)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        create_nested_structure(temp_dir.path(), d, br);
                        temp_dir
                    },
                    |temp_dir| {
                        let path = temp_dir.path().to_path_buf();
                        let output = Command::new("cargo")
                            .args(&["run", "--release", "--"])
                            .arg(&path)
                            .output()
                            .expect("Failed to run fast-rm");

                        black_box(output);
                    },
                );
            },
        );
    }

    group.finish();
}

/// Compare fast-rm vs system rm -rf
fn bench_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fast_rm_vs_system_rm");

    let size = 1000;
    group.throughput(Throughput::Elements(size as u64));

    // Benchmark fast-rm
    group.bench_function("fast_rm_1000", |b| {
        b.iter_with_setup(
            || {
                let temp_dir = TempDir::new().unwrap();
                create_flat_structure(temp_dir.path(), size);
                temp_dir
            },
            |temp_dir| {
                let path = temp_dir.path().to_path_buf();
                let output = Command::new("cargo")
                    .args(&["run", "--release", "--"])
                    .arg(&path)
                    .output()
                    .expect("Failed to run fast-rm");

                black_box(output);
            },
        );
    });

    // Benchmark system rm -rf
    group.bench_function("system_rm_1000", |b| {
        b.iter_with_setup(
            || {
                let temp_dir = TempDir::new().unwrap();
                create_flat_structure(temp_dir.path(), size);
                temp_dir
            },
            |temp_dir| {
                let path = temp_dir.path().to_path_buf();
                let output = Command::new("rm")
                    .args(&["-rf", path.to_str().unwrap()])
                    .output()
                    .expect("Failed to run rm");

                black_box(output);
            },
        );
    });

    group.finish();
}

/// Benchmark different thread configurations
fn bench_thread_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("thread_scaling");

    let size = 2000;
    group.throughput(Throughput::Elements(size as u64));

    for threads in [1, 2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}threads", threads)),
            threads,
            |b, &threads| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        create_flat_structure(temp_dir.path(), size);
                        temp_dir
                    },
                    |temp_dir| {
                        let path = temp_dir.path().to_path_buf();
                        let output = Command::new("cargo")
                            .args(&[
                                "run",
                                "--release",
                                "--",
                                "--scan-threads",
                                &threads.to_string(),
                                "--delete-threads",
                                &threads.to_string(),
                            ])
                            .arg(&path)
                            .output()
                            .expect("Failed to run fast-rm");

                        black_box(output);
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_fast_rm_flat,
    bench_fast_rm_nested,
    bench_comparison,
    bench_thread_scaling
);
criterion_main!(benches);
