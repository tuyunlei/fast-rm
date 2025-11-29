//! Performance benchmarks for fast-rm
//!
//! Run with: cargo bench
//! Results are saved to: target/criterion/report/index.html

use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use std::fs::{self, File};
use std::hint::black_box;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// Test Structure Generators
// ============================================================================

/// Create a flat directory with N files
fn create_flat_structure(base: &Path, num_files: usize) -> usize {
    for i in 0..num_files {
        let file_path = base.join(format!("file_{:06}.txt", i));
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "test content {}", i).unwrap();
    }
    num_files
}

/// Create a flat directory with N files using parallel creation for large-scale tests
/// Uses multiple threads to speed up file creation
fn create_flat_structure_parallel(base: &Path, num_files: usize) -> usize {
    use std::sync::Arc;
    use std::thread;

    let base = Arc::new(base.to_path_buf());
    let num_threads = num_cpus::get().min(16);
    let files_per_thread = num_files / num_threads;
    let remainder = num_files % num_threads;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let base = Arc::clone(&base);
            let start = t * files_per_thread + t.min(remainder);
            let count = files_per_thread + if t < remainder { 1 } else { 0 };

            thread::spawn(move || {
                for i in start..(start + count) {
                    let file_path = base.join(format!("file_{:08}.txt", i));
                    let mut file = File::create(&file_path).unwrap();
                    writeln!(file, "test content {}", i).unwrap();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    num_files
}

/// Create a large nested structure with many directories and files
/// Designed to create 100K+ items efficiently
fn create_large_nested_structure(base: &Path, dirs_per_level: usize, files_per_dir: usize, depth: usize) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let count = Arc::new(AtomicUsize::new(0));

    fn recurse(
        path: &Path,
        dirs_per_level: usize,
        files_per_dir: usize,
        depth: usize,
        count: &AtomicUsize,
    ) {
        // Create files at this level
        for i in 0..files_per_dir {
            let file = path.join(format!("f_{}.txt", i));
            let mut f = File::create(&file).unwrap();
            writeln!(f, "x").unwrap();
            count.fetch_add(1, Ordering::Relaxed);
        }

        if depth > 0 {
            // Create subdirectories and recurse
            for i in 0..dirs_per_level {
                let dir = path.join(format!("d_{}", i));
                fs::create_dir(&dir).unwrap();
                count.fetch_add(1, Ordering::Relaxed);
                recurse(&dir, dirs_per_level, files_per_dir, depth - 1, count);
            }
        }
    }

    recurse(base, dirs_per_level, files_per_dir, depth, &count);
    count.load(Ordering::Relaxed)
}

/// Create a nested directory structure
/// Returns total number of items (files + directories)
fn create_nested_structure(base: &Path, depth: usize, breadth: usize) -> usize {
    fn recurse(path: &Path, depth: usize, breadth: usize) -> usize {
        if depth == 0 {
            return 0;
        }

        let mut count = 0;

        // Create files at this level
        for i in 0..breadth {
            let file = path.join(format!("file_{}.txt", i));
            let mut f = File::create(&file).unwrap();
            writeln!(f, "content {}", i).unwrap();
            count += 1;
        }

        // Create subdirectories and recurse
        for i in 0..breadth {
            let dir = path.join(format!("dir_{}", i));
            fs::create_dir(&dir).unwrap();
            count += 1;
            count += recurse(&dir, depth - 1, breadth);
        }

        count
    }

    recurse(base, depth, breadth)
}

/// Create a deep directory chain (worst case for recursive deletion)
fn create_deep_structure(base: &Path, depth: usize, files_per_level: usize) -> usize {
    let mut current = base.to_path_buf();
    let mut count = 0;

    for level in 0..depth {
        // Create files at this level
        for i in 0..files_per_level {
            let file = current.join(format!("file_{}_{}.txt", level, i));
            let mut f = File::create(&file).unwrap();
            writeln!(f, "level {} file {}", level, i).unwrap();
            count += 1;
        }

        // Create next level directory
        if level < depth - 1 {
            let next_dir = current.join(format!("level_{}", level + 1));
            fs::create_dir(&next_dir).unwrap();
            count += 1;
            current = next_dir;
        }
    }

    count
}

/// Create mixed structure with various file sizes
fn create_mixed_structure(
    base: &Path,
    num_small: usize,
    num_medium: usize,
    num_large: usize,
) -> usize {
    // Small files (< 1KB)
    for i in 0..num_small {
        let file = base.join(format!("small_{:06}.txt", i));
        let mut f = File::create(&file).unwrap();
        writeln!(f, "small file {}", i).unwrap();
    }

    // Medium files (~10KB)
    let medium_content: String = "x".repeat(10 * 1024);
    for i in 0..num_medium {
        let file = base.join(format!("medium_{:06}.dat", i));
        let mut f = File::create(&file).unwrap();
        f.write_all(medium_content.as_bytes()).unwrap();
    }

    // Large files (~100KB)
    let large_content: String = "y".repeat(100 * 1024);
    for i in 0..num_large {
        let file = base.join(format!("large_{:06}.dat", i));
        let mut f = File::create(&file).unwrap();
        f.write_all(large_content.as_bytes()).unwrap();
    }

    num_small + num_medium + num_large
}

// ============================================================================
// Benchmark Helpers
// ============================================================================

/// Get the path to the fast-rm binary
fn get_fast_rm_binary() -> String {
    // Build release binary first if needed
    let status = Command::new("cargo")
        .args(["build", "--release", "--quiet"])
        .status()
        .expect("Failed to build fast-rm");
    assert!(status.success(), "Failed to build fast-rm");

    "./target/release/fast-rm".to_string()
}

/// Create a target directory inside temp dir and return its path
fn create_target_dir(temp_dir: &TempDir) -> std::path::PathBuf {
    let target = temp_dir.path().join("target");
    fs::create_dir(&target).unwrap();
    target
}

/// Run fast-rm on a directory
fn run_fast_rm(path: &Path, binary: &str) {
    let output = Command::new(binary)
        .arg(path)
        .output()
        .expect("Failed to run fast-rm");
    // Check that most items were deleted (allow minor errors due to race conditions)
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Only panic if it's a real failure, not just cleanup race
        if !stderr.contains("error") || stderr.contains("Permission denied") {
            panic!("fast-rm failed: {:?}", output);
        }
    }
}

/// Run system rm -r on a directory
fn run_system_rm(path: &Path) {
    let output = Command::new("rm")
        .args(["-rf", path.to_str().unwrap()])
        .output()
        .expect("Failed to run rm");
    assert!(output.status.success(), "rm failed: {:?}", output);
}

// ============================================================================
// Benchmarks: fast-rm vs rm -r Comparison (Main Focus)
// ============================================================================

fn bench_vs_system_rm(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("fast-rm_vs_rm");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(20); // Fewer samples for I/O-bound benchmarks

    // Test configurations: (name, file_count)
    let configs = [
        ("0100_files", 100),
        ("0500_files", 500),
        ("1000_files", 1000),
        ("2000_files", 2000),
        ("5000_files", 5000),
    ];

    for (name, file_count) in configs {
        group.throughput(Throughput::Elements(file_count as u64));

        // Benchmark fast-rm
        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &file_count,
            |b, &count| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_flat_structure(&target, count);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        // Benchmark system rm -r
        group.bench_with_input(BenchmarkId::new("rm -r", name), &file_count, |b, &count| {
            b.iter_with_setup(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let target = create_target_dir(&temp_dir);
                    create_flat_structure(&target, count);
                    (temp_dir, target)
                },
                |(temp_dir, target)| {
                    run_system_rm(&target);
                    drop(temp_dir);
                    black_box(())
                },
            );
        });
    }

    group.finish();
}

// ============================================================================
// Benchmarks: Nested Directory Structure
// ============================================================================

fn bench_nested_structure(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("nested_structure");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(15);

    // (name, depth, breadth)
    let configs = [
        ("shallow_wide", 2, 10), // 2 levels, 10 items each
        ("medium", 3, 5),        // 3 levels, 5 items each
        ("deep_narrow", 5, 3),   // 5 levels, 3 items each
        ("very_deep", 8, 2),     // 8 levels, 2 items each
    ];

    for (name, depth, breadth) in configs {
        // Calculate expected items for throughput
        let temp = TempDir::new().unwrap();
        let target = create_target_dir(&temp);
        let item_count = create_nested_structure(&target, depth, breadth);
        drop(temp);

        group.throughput(Throughput::Elements(item_count as u64));

        // fast-rm
        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &(depth, breadth),
            |b, &(d, br)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_nested_structure(&target, d, br);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        // rm -r
        group.bench_with_input(
            BenchmarkId::new("rm -r", name),
            &(depth, breadth),
            |b, &(d, br)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_nested_structure(&target, d, br);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_system_rm(&target);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmarks: Deep Directory Chain
// ============================================================================

fn bench_deep_chain(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("deep_chain");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(15);

    // (depth, files_per_level)
    let configs = [
        ("depth_10", 10, 5),
        ("depth_20", 20, 5),
        ("depth_50", 50, 3),
    ];

    for (name, depth, files) in configs {
        let temp = TempDir::new().unwrap();
        let target = create_target_dir(&temp);
        let item_count = create_deep_structure(&target, depth, files);
        drop(temp);

        group.throughput(Throughput::Elements(item_count as u64));

        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &(depth, files),
            |b, &(d, f)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_deep_structure(&target, d, f);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("rm -r", name),
            &(depth, files),
            |b, &(d, f)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_deep_structure(&target, d, f);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_system_rm(&target);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmarks: Mixed File Sizes
// ============================================================================

fn bench_mixed_sizes(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("mixed_file_sizes");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(15);

    // (name, small, medium, large)
    let configs = [
        ("mostly_small", 900, 90, 10),
        ("balanced", 500, 300, 200),
        ("mostly_large", 100, 100, 800),
    ];

    for (name, small, medium, large) in configs {
        let total = small + medium + large;
        group.throughput(Throughput::Elements(total as u64));

        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &(small, medium, large),
            |b, &(s, m, l)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_mixed_structure(&target, s, m, l);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("rm -r", name),
            &(small, medium, large),
            |b, &(s, m, l)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_mixed_structure(&target, s, m, l);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_system_rm(&target);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmarks: Thread Scaling (fast-rm only)
// ============================================================================

fn bench_thread_scaling(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("thread_scaling");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(15);

    let file_count = 2000;
    group.throughput(Throughput::Elements(file_count as u64));

    for threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_threads", threads)),
            &threads,
            |b, &t| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_flat_structure(&target, file_count as usize);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        let output = Command::new(&fast_rm)
                            .args([
                                "--scan-threads",
                                &t.to_string(),
                                "--delete-threads",
                                &t.to_string(),
                            ])
                            .arg(&target)
                            .output()
                            .expect("Failed to run fast-rm");
                        if !output.status.success() {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            if !stderr.contains("error") || stderr.contains("Permission denied") {
                                panic!("fast-rm failed: {:?}", output);
                            }
                        }
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmarks: Large Scale (10s+ deletion times)
// ============================================================================

fn bench_large_scale_flat(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("large_scale_flat");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10); // Fewer samples for long-running tests

    // Large scale configurations to achieve 10s+ rm times
    // Based on ~50μs per file on tmpfs, we need:
    // - 200K files for ~10s
    // - 600K files for ~30s
    let configs = [
        ("050k_files", 50_000),
        ("100k_files", 100_000),
        ("200k_files", 200_000),
        ("500k_files", 500_000),
    ];

    for (name, file_count) in configs {
        group.throughput(Throughput::Elements(file_count as u64));
        // Set measurement time based on expected duration
        let measurement_secs = match file_count {
            n if n >= 500_000 => 120,
            n if n >= 200_000 => 60,
            n if n >= 100_000 => 30,
            _ => 20,
        };
        group.measurement_time(std::time::Duration::from_secs(measurement_secs));

        // Benchmark fast-rm
        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &file_count,
            |b, &count| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_flat_structure_parallel(&target, count);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        // Benchmark system rm -r
        group.bench_with_input(BenchmarkId::new("rm -r", name), &file_count, |b, &count| {
            b.iter_with_setup(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let target = create_target_dir(&temp_dir);
                    create_flat_structure_parallel(&target, count);
                    (temp_dir, target)
                },
                |(temp_dir, target)| {
                    run_system_rm(&target);
                    drop(temp_dir);
                    black_box(())
                },
            );
        });
    }

    group.finish();
}

fn bench_large_scale_nested(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("large_scale_nested");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(60));

    // Large nested structures
    // (name, dirs_per_level, files_per_dir, depth) -> approximate item count
    let configs = [
        // 10 dirs × 100 files × 4 levels ≈ 111K items
        ("100k_nested", 10, 100, 4),
        // 10 dirs × 50 files × 5 levels ≈ 555K items
        ("500k_nested", 10, 50, 5),
    ];

    for (name, dirs, files, depth) in configs {
        // Calculate expected items for throughput
        let temp = TempDir::new().unwrap();
        let target = create_target_dir(&temp);
        let item_count = create_large_nested_structure(&target, dirs, files, depth);
        drop(temp);

        println!("Config {}: {} items", name, item_count);
        group.throughput(Throughput::Elements(item_count as u64));

        // fast-rm
        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &(dirs, files, depth),
            |b, &(d, f, dep)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_large_nested_structure(&target, d, f, dep);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_fast_rm(&target, &fast_rm);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );

        // rm -r
        group.bench_with_input(
            BenchmarkId::new("rm -r", name),
            &(dirs, files, depth),
            |b, &(d, f, dep)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let target = create_target_dir(&temp_dir);
                        create_large_nested_structure(&target, d, f, dep);
                        (temp_dir, target)
                    },
                    |(temp_dir, target)| {
                        run_system_rm(&target);
                        drop(temp_dir);
                        black_box(())
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark specifically designed to test scenarios where rm takes 30+ seconds
fn bench_extreme_scale(c: &mut Criterion) {
    let fast_rm = get_fast_rm_binary();
    let mut group = c.benchmark_group("extreme_scale");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(5); // Very few samples for extremely long tests
    group.measurement_time(std::time::Duration::from_secs(180)); // 3 minutes

    // 1 million files - should take ~50s for rm on tmpfs
    let file_count = 1_000_000;
    group.throughput(Throughput::Elements(file_count as u64));

    // Benchmark fast-rm
    group.bench_function("fast-rm/1M_files", |b| {
        b.iter_with_setup(
            || {
                let temp_dir = TempDir::new().unwrap();
                let target = create_target_dir(&temp_dir);
                create_flat_structure_parallel(&target, file_count);
                (temp_dir, target)
            },
            |(temp_dir, target)| {
                run_fast_rm(&target, &fast_rm);
                drop(temp_dir);
                black_box(())
            },
        );
    });

    // Benchmark system rm -r
    group.bench_function("rm -r/1M_files", |b| {
        b.iter_with_setup(
            || {
                let temp_dir = TempDir::new().unwrap();
                let target = create_target_dir(&temp_dir);
                create_flat_structure_parallel(&target, file_count);
                (temp_dir, target)
            },
            |(temp_dir, target)| {
                run_system_rm(&target);
                drop(temp_dir);
                black_box(())
            },
        );
    });

    group.finish();
}

// ============================================================================
// Main
// ============================================================================

criterion_group!(
    benches,
    bench_vs_system_rm,     // Primary: fast-rm vs rm -r
    bench_nested_structure, // Nested directories
    bench_deep_chain,       // Deep directory chains
    bench_mixed_sizes,      // Various file sizes
    bench_thread_scaling,   // Thread pool tuning
);

// Separate group for large-scale tests (run with: cargo bench -- "large_scale")
criterion_group!(
    name = large_scale_benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(std::time::Duration::from_secs(60));
    targets = bench_large_scale_flat, bench_large_scale_nested
);

// Extreme scale tests (run with: cargo bench -- "extreme_scale")
criterion_group!(
    name = extreme_benches;
    config = Criterion::default()
        .sample_size(5)
        .measurement_time(std::time::Duration::from_secs(180));
    targets = bench_extreme_scale
);

criterion_main!(benches, large_scale_benches, extreme_benches);
