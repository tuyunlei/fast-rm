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

/// Run fast-rm on a directory
fn run_fast_rm(path: &Path, binary: &str) {
    let output = Command::new(binary)
        .arg(path)
        .output()
        .expect("Failed to run fast-rm");
    assert!(output.status.success(), "fast-rm failed: {:?}", output);
}

/// Run system rm -r on a directory
fn run_system_rm(path: &Path) {
    let output = Command::new("rm")
        .args(["-r", path.to_str().unwrap()])
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
                        create_flat_structure(temp_dir.path(), count);
                        temp_dir
                    },
                    |temp_dir| {
                        run_fast_rm(temp_dir.path(), &fast_rm);
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
                    create_flat_structure(temp_dir.path(), count);
                    temp_dir
                },
                |temp_dir| {
                    run_system_rm(temp_dir.path());
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
        let item_count = create_nested_structure(temp.path(), depth, breadth);
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
                        create_nested_structure(temp_dir.path(), d, br);
                        temp_dir
                    },
                    |temp_dir| {
                        run_fast_rm(temp_dir.path(), &fast_rm);
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
                        create_nested_structure(temp_dir.path(), d, br);
                        temp_dir
                    },
                    |temp_dir| {
                        run_system_rm(temp_dir.path());
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
        let item_count = create_deep_structure(temp.path(), depth, files);
        drop(temp);

        group.throughput(Throughput::Elements(item_count as u64));

        group.bench_with_input(
            BenchmarkId::new("fast-rm", name),
            &(depth, files),
            |b, &(d, f)| {
                b.iter_with_setup(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        create_deep_structure(temp_dir.path(), d, f);
                        temp_dir
                    },
                    |temp_dir| {
                        run_fast_rm(temp_dir.path(), &fast_rm);
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
                        create_deep_structure(temp_dir.path(), d, f);
                        temp_dir
                    },
                    |temp_dir| {
                        run_system_rm(temp_dir.path());
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
                        create_mixed_structure(temp_dir.path(), s, m, l);
                        temp_dir
                    },
                    |temp_dir| {
                        run_fast_rm(temp_dir.path(), &fast_rm);
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
                        create_mixed_structure(temp_dir.path(), s, m, l);
                        temp_dir
                    },
                    |temp_dir| {
                        run_system_rm(temp_dir.path());
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
                        create_flat_structure(temp_dir.path(), file_count as usize);
                        temp_dir
                    },
                    |temp_dir| {
                        let output = Command::new(&fast_rm)
                            .args([
                                "--scan-threads",
                                &t.to_string(),
                                "--delete-threads",
                                &t.to_string(),
                            ])
                            .arg(temp_dir.path())
                            .output()
                            .expect("Failed to run fast-rm");
                        assert!(output.status.success());
                        black_box(())
                    },
                );
            },
        );
    }

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

criterion_main!(benches);
