# fast-rm

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A high-performance, concurrent file and directory removal tool written in Rust.

[中文文档](./README_zh-CN.md)

## Features

- **Two-Pool Architecture** - Separate scanner and deleter thread pools for maximum throughput
- **Lock-Free Design** - Atomic counters and crossbeam channels eliminate mutex contention
- **Real-Time Progress** - Beautiful TUI with live progress tracking, deletion speed, and queue depth
- **Safe by Default** - Path overlap detection, symlink handling, and dry-run mode
- **Fine-Grained Control** - Independent tuning of scanner and deleter thread counts
- **Error Resilient** - Continue-on-error mode to handle failures gracefully

## Installation

### From Source

```bash
git clone https://github.com/yourusername/fast-rm.git
cd fast-rm
cargo build --release
```

The binary will be available at `target/release/fast-rm`.

## Usage

```bash
# Basic usage
fast-rm <paths>...

# Dry run (show what would be deleted)
fast-rm -n <paths>

# Verbose output (-v for standard, -vv for detailed)
fast-rm -v <paths>

# Set thread count for both pools
fast-rm -j 8 <paths>

# Fine-grained thread control
fast-rm --scan-threads 4 --delete-threads 8 <paths>

# Continue on errors
fast-rm -c <paths>

# Combine options
fast-rm -v -n -c <paths>
```

## Options

| Option | Short | Description |
|--------|-------|-------------|
| `--verbose` | `-v` | Increase verbosity (-v: standard, -vv: detailed) |
| `--dry-run` | `-n` | Show what would be deleted without removing |
| `--threads` | `-j` | Number of threads for both pools (default: CPU cores) |
| `--scan-threads` | | Number of scanner threads (overrides -j) |
| `--delete-threads` | | Number of deleter threads (overrides -j) |
| `--continue-on-error` | `-c` | Continue processing after errors |

## Architecture

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  Scanner Pool   │────▶│  Adaptive Queue  │────▶│  Deleter Pool   │
│  (Rayon-based)  │     │  (Lock-free)     │     │  (Worker threads)│
└─────────────────┘     └──────────────────┘     └─────────────────┘
        │                        │                        │
        ▼                        ▼                        ▼
   Parallel DFS           MPMC Channel              Concurrent
   Traversal              with Backpressure         Deletion
```

### Why Two Pools?

1. **Accurate Progress** - Scanning completes first, providing exact item counts
2. **Maximum Throughput** - Scanners and deleters work simultaneously
3. **Independent Tuning** - Optimize for your I/O and CPU characteristics
4. **Queue Visibility** - Real-time queue depth shows pipeline health

## Performance

| Metric | Value |
|--------|-------|
| Throughput | 1,948 - 4,465 items/sec |
| Thread Scaling | 25% improvement with 4 threads |
| Memory Overhead | Minimal (lock-free design) |

Best suited for:
- Large directory trees (> 10,000 items)
- When progress visibility is needed
- Safety-critical operations

See [BENCHMARK.md](./BENCHMARK.md) for detailed performance analysis.

## Safety Features

- **Path Overlap Detection** - Prevents concurrent deletion of nested paths
- **Symlink Handling** - Uses `symlink_metadata()` to avoid following broken symlinks
- **Dry-Run Mode** - Test deletions safely before executing
- **Continue-on-Error** - Handle permission errors without stopping

## Development

```bash
# Run tests
cargo test

# Run linter
cargo clippy

# Format code
cargo fmt

# Run benchmarks
cargo bench
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
