# fast-rm Performance Benchmarks

This document provides comprehensive performance analysis of fast-rm compared to system `rm -r`.

## Test Environment

| Component | Details |
|-----------|---------|
| **OS** | Linux x86_64 (Kernel 4.4.0) |
| **CPU** | 16 cores |
| **Memory** | 13 GB |
| **Filesystem** | tmpfs (RAM-backed) |
| **Rust** | 1.70+ (release build with optimizations) |
| **Benchmark Tool** | Criterion 0.7.0 |

## Running Benchmarks

```bash
# Run all benchmarks (takes 5-10 minutes)
cargo bench

# Run specific benchmark group
cargo bench -- "fast-rm_vs_rm"
cargo bench -- "nested_structure"
cargo bench -- "thread_scaling"

# View HTML report
open target/criterion/report/index.html
```

## Benchmark Results

### 1. fast-rm vs rm -r (Flat Structure)

Primary comparison benchmark with varying file counts.

| Files | fast-rm Time | fast-rm Throughput | rm -r Time | rm -r Throughput | Ratio |
|-------|--------------|-------------------|------------|------------------|-------|
| 100 | 170.21 ms | 587 elem/s | 19.06 ms | 5,247 elem/s | 0.11x |
| 500 | 191.67 ms | 2,609 elem/s | 54.99 ms | 9,092 elem/s | 0.29x |
| 1,000 | 220.49 ms | 4,535 elem/s | 96.93 ms | 10,317 elem/s | 0.44x |
| 2,000 | 351.49 ms | 5,690 elem/s | 160.27 ms | 12,479 elem/s | 0.46x |
| 5,000 | 631.50 ms | 7,917 elem/s | ~250 ms | ~20,000 elem/s | 0.40x |

**Key Findings:**

1. **Fixed Startup Overhead**: fast-rm has ~170ms constant overhead regardless of file count
   - TUI initialization: ~50-100ms
   - Thread pool creation: ~50ms
   - Binary startup: ~20ms

2. **Throughput Scaling**: fast-rm throughput increases with file count
   - 100 files: 587 elem/s
   - 5,000 files: 7,917 elem/s
   - **13.5x improvement** as amortized overhead decreases

3. **rm -r Advantage**: System rm is consistently faster
   - Minimal startup overhead
   - Single-threaded but highly optimized
   - Direct syscalls without progress tracking

### 2. Nested Directory Structure

Tests hierarchical directory trees with varying depth and breadth.

| Structure | Depth | Breadth | Items | fast-rm | rm -r |
|-----------|-------|---------|-------|---------|-------|
| shallow_wide | 2 | 10 | 220 | ~180ms | ~25ms |
| medium | 3 | 5 | 155 | ~175ms | ~20ms |
| deep_narrow | 5 | 3 | 363 | ~190ms | ~35ms |
| very_deep | 8 | 2 | 510 | ~200ms | ~45ms |

**Observations:**
- Nested structures show similar patterns to flat structures
- fast-rm's parallel scanning helps with wide structures
- Deep structures have slightly higher overhead due to directory traversal

### 3. Deep Directory Chain

Worst-case scenario: single deep chain of directories.

| Test | Depth | Files/Level | Total Items | fast-rm | rm -r |
|------|-------|-------------|-------------|---------|-------|
| depth_10 | 10 | 5 | 59 | ~175ms | ~15ms |
| depth_20 | 20 | 5 | 119 | ~180ms | ~18ms |
| depth_50 | 50 | 3 | 199 | ~190ms | ~25ms |

**Observations:**
- Deep chains are dominated by startup overhead
- Directory ordering (parent-after-children) works correctly
- No performance degradation with depth

### 4. Mixed File Sizes

Tests impact of file size on deletion performance.

| Test | Small (<1KB) | Medium (~10KB) | Large (~100KB) | fast-rm | rm -r |
|------|--------------|----------------|----------------|---------|-------|
| mostly_small | 900 | 90 | 10 | ~220ms | ~95ms |
| balanced | 500 | 300 | 200 | ~230ms | ~100ms |
| mostly_large | 100 | 100 | 800 | ~250ms | ~120ms |

**Observations:**
- File size has minimal impact on deletion time
- `unlink()` syscall is fast regardless of file size
- Larger files show slight overhead from metadata operations

### 5. Thread Scaling

Tests fast-rm performance with different thread counts (2,000 files).

| Threads | Time | Throughput | Scaling |
|---------|------|------------|---------|
| 1 | 380ms | 5,263 elem/s | 1.00x (baseline) |
| 2 | 355ms | 5,634 elem/s | 1.07x |
| 4 | 340ms | 5,882 elem/s | 1.12x |
| 8 | 350ms | 5,714 elem/s | 1.09x |

**Observations:**
- Modest scaling due to I/O-bound nature
- Sweet spot around 4 threads for this workload
- Diminishing returns beyond 4 threads
- Thread coordination overhead visible at 8 threads

## Performance Analysis

### Overhead Breakdown

| Component | Time | Percentage |
|-----------|------|------------|
| Binary startup | ~20ms | 12% |
| TUI initialization | ~80ms | 47% |
| Thread pool creation | ~50ms | 29% |
| Queue setup | ~10ms | 6% |
| Actual deletion | ~10ms | 6% |

*Based on 100-file benchmark where overhead dominates*

### Why fast-rm is Slower

1. **Process Overhead**: Rust binary with dependencies vs simple C utility
2. **TUI Rendering**: indicatif + crossterm initialization
3. **Two-Pool Architecture**: Queue coordination between scanner and deleter
4. **Progress Tracking**: Atomic counters and channel operations
5. **Safety Features**: Path validation, overlap detection

### Where fast-rm Excels

Despite being slower, fast-rm provides value in:

| Feature | Benefit |
|---------|---------|
| **Progress Visibility** | Know exactly what's being deleted and how fast |
| **Queue Depth** | See if deletion is keeping up with scanning |
| **Error Reporting** | Immediate feedback on permission issues |
| **Dry-Run Mode** | Test before destructive operations |
| **Continue-on-Error** | Don't stop on first failure |
| **Path Overlap Detection** | Prevent race conditions |

## Benchmark Groups

### Test Configurations

| Group | Description |
|-------|-------------|
| `fast-rm_vs_rm` | Primary comparison: fast-rm vs system rm -r |
| `nested_structure` | Hierarchical directory trees |
| `deep_chain` | Deep single-path directories |
| `mixed_file_sizes` | Various file sizes impact |
| `thread_scaling` | Thread count optimization |

### Interpreting Criterion Output

```
fast-rm_vs_rm/fast-rm/1000_files
                        time:   [220.20 ms 220.49 ms 220.78 ms]
                        thrpt:  [4.5294 Kelem/s 4.5354 Kelem/s 4.5413 Kelem/s]
```

- **time**: [lower bound, estimate, upper bound] with 95% confidence
- **thrpt**: Throughput in elements (files) per second
- **change**: Comparison with previous run (if available)

## Recommendations

### Use fast-rm When

- Deleting large directories (> 5,000 items) where progress visibility matters
- Working with production data requiring dry-run first
- Need to continue past errors
- Want to monitor deletion speed and queue health

### Use rm -r When

- Small directories (< 500 items)
- Maximum speed is critical
- Scripting scenarios without user interaction
- Simple, fire-and-forget deletion

## Reproducing Results

For consistent benchmarks:

```bash
# 1. Build release binary
cargo build --release

# 2. Drop filesystem caches (Linux)
sync && echo 3 | sudo tee /proc/sys/vm/drop_caches

# 3. Run benchmarks
cargo bench

# 4. Compare with baseline
cargo bench -- --save-baseline main
# ... make changes ...
cargo bench -- --baseline main
```

## Architecture Impact

fast-rm's two-pool architecture affects benchmarks:

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Scanners   │────▶│    Queue    │────▶│  Deleters   │
│  (Rayon)    │     │  (Lock-free)│     │  (Workers)  │
└─────────────┘     └─────────────┘     └─────────────┘
```

**Overhead sources:**
- Process startup: ~20ms
- TUI initialization: ~80ms
- Thread pool creation: ~50ms
- Queue coordination: < 1% of runtime

**Architectural benefits:**
- Accurate progress tracking (scan completes before deletion totals known)
- Error resilience (continue-on-error mode)
- Configurable parallelism (independent scan/delete thread pools)

## Contributing

When adding new benchmarks:

1. Use `SamplingMode::Flat` for I/O-bound tests
2. Keep `sample_size` low (15-20) to reduce runtime
3. Include both fast-rm and `rm -r` for comparison
4. Document expected behavior in this file

---

*Last updated: 2024*
*Tool version: fast-rm 0.1.0*
*Architecture: Two-pool scan/delete with lock-free coordination*
