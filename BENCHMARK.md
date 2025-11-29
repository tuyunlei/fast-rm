# fast-rm Performance Benchmarks

This document describes the benchmark suite for fast-rm and provides guidance on interpreting results.

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

## Benchmark Groups

### 1. fast-rm vs rm -r (`fast-rm_vs_rm`)

**Primary benchmark** - Direct comparison with system `rm -r` command.

| Test Case | Files | Description |
|-----------|-------|-------------|
| 0100_files | 100 | Small directory, measures startup overhead |
| 0500_files | 500 | Medium directory |
| 1000_files | 1,000 | Standard test case |
| 2000_files | 2,000 | Larger dataset |
| 5000_files | 5,000 | Stress test |

**What to expect:**
- `rm -r` is typically faster for small datasets (< 500 files) due to fast-rm's startup overhead
- fast-rm becomes competitive or faster at larger scales where parallelism pays off
- Both are ultimately I/O-bound by filesystem performance

### 2. Nested Directory Structure (`nested_structure`)

Tests performance on hierarchical directory trees.

| Test Case | Structure | Description |
|-----------|-----------|-------------|
| shallow_wide | depth=2, breadth=10 | Wide, shallow tree |
| medium | depth=3, breadth=5 | Balanced structure |
| deep_narrow | depth=5, breadth=3 | Deeper hierarchy |
| very_deep | depth=8, breadth=2 | Very deep nesting |

**What to expect:**
- Nested structures benefit from fast-rm's parallel scanning
- Deeper structures show more variance due to directory traversal patterns

### 3. Deep Directory Chain (`deep_chain`)

Worst-case scenario for recursive deletion - a single deep chain.

| Test Case | Depth | Files/Level |
|-----------|-------|-------------|
| depth_10 | 10 | 5 |
| depth_20 | 20 | 5 |
| depth_50 | 50 | 3 |

**What to expect:**
- Tests the scanner's depth-first traversal efficiency
- Verifies correct parent-after-children deletion ordering

### 4. Mixed File Sizes (`mixed_file_sizes`)

Tests impact of file size on deletion performance.

| Test Case | Small (<1KB) | Medium (~10KB) | Large (~100KB) |
|-----------|--------------|----------------|----------------|
| mostly_small | 900 | 90 | 10 |
| balanced | 500 | 300 | 200 |
| mostly_large | 100 | 100 | 800 |

**What to expect:**
- File size has minimal impact on deletion time (unlink is fast)
- Larger files may show slight overhead from filesystem metadata updates

### 5. Thread Scaling (`thread_scaling`)

Tests parallel efficiency with different thread counts.

| Threads | Configuration |
|---------|---------------|
| 1 | Single-threaded baseline |
| 2 | Minimal parallelism |
| 4 | Sweet spot for most systems |
| 8 | High parallelism |

**What to expect:**
- Diminishing returns beyond 4 threads for most workloads
- I/O-bound nature limits scaling benefits
- Thread coordination overhead visible at high thread counts

## Interpreting Results

### Criterion Output

```
fast-rm_vs_rm/fast-rm/1000_files
                        time:   [512.3 ms 518.7 ms 525.1 ms]
                        thrpt:  [1904 elem/s 1928 elem/s 1952 elem/s]
```

- **time**: [lower bound, estimate, upper bound] with 95% confidence
- **thrpt**: Throughput in elements (files) per second
- **change**: Comparison with previous run (if available)

### Performance Factors

Results are affected by:

1. **Filesystem type**: ext4, XFS, ZFS, etc. have different performance characteristics
2. **Storage medium**: SSD vs HDD significantly impacts I/O-bound operations
3. **Kernel caching**: Warm cache vs cold cache affects results
4. **System load**: Other processes competing for I/O
5. **File system fragmentation**: Affects sequential access patterns

### Recommendations

| Scenario | Recommendation |
|----------|----------------|
| Small datasets (< 500 files) | Use system `rm -r` for simplicity |
| Large datasets (> 1,000 files) | fast-rm provides progress visibility |
| Need progress tracking | fast-rm with `-v` flag |
| Maximum speed | System `rm -r` (no TUI overhead) |
| Safety critical | fast-rm with `-n` (dry-run first) |

## Architecture Impact

fast-rm's two-pool architecture affects benchmarks:

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Scanners   │────▶│    Queue    │────▶│  Deleters   │
│  (Rayon)    │     │  (Lock-free)│     │  (Workers)  │
└─────────────┘     └─────────────┘     └─────────────┘
```

**Overhead sources:**
- Process startup: ~50-100ms
- TUI initialization: ~20-50ms
- Queue coordination: < 1% of total time

**Where fast-rm excels:**
- Accurate progress tracking (scan completes before deletion totals known)
- Error resilience (continue-on-error mode)
- Configurable parallelism (independent scan/delete thread pools)

## Reproducing Results

For consistent benchmarks:

```bash
# 1. Close unnecessary applications
# 2. Disable CPU frequency scaling (if possible)
sudo cpupower frequency-set -g performance

# 3. Drop filesystem caches
sync && echo 3 | sudo tee /proc/sys/vm/drop_caches

# 4. Run benchmarks
cargo bench

# 5. Compare with baseline
cargo bench -- --save-baseline main
# ... make changes ...
cargo bench -- --baseline main
```

## Contributing

When adding new benchmarks:

1. Use `SamplingMode::Flat` for I/O-bound tests
2. Keep `sample_size` low (15-20) to reduce runtime
3. Include both fast-rm and `rm -r` for comparison
4. Document expected behavior in this file
