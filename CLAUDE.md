# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`fast-rm` is a fast, concurrent file and directory removal tool written in Rust. It uses parallel processing via the `rayon` crate to efficiently delete files and directories.

## Development Commands

### Building
```bash
cargo build          # Debug build
cargo build --release  # Optimized release build
```

### Running
```bash
cargo run -- <paths>              # Run with paths to remove
cargo run -- --help               # Show help
cargo run -- -v <paths>           # Verbose mode
cargo run -- -n <paths>           # Dry run (show what would be deleted)
cargo run -- --dry-run <paths>    # Same as -n
```

### Testing
```bash
cargo test           # Run all tests
cargo test <name>    # Run specific test
```

### Code Quality
```bash
cargo clippy         # Run linter
cargo fmt            # Format code
cargo check          # Fast compilation check without producing binary
```

## Architecture

### Core Design

The application is a single-file CLI tool (`src/main.rs`) with these key components:

1. **CLI Interface** (`Cli` struct, lines 8-31)
   - Uses `clap` with derive macros for argument parsing
   - Supports: `--force`, `--verbose`, `--dry-run` flags
   - Accepts multiple paths via `Vec<PathBuf>`

2. **Main Entry Point** (`main()`, lines 33-91)
   - Parses CLI arguments
   - Uses **Rayon's parallel iterator** (`par_iter()`) to process multiple top-level paths concurrently
   - Each path is processed independently in parallel

3. **Recursive Removal** (`fast_remove()`, lines 94-252)
   - **Handles symlinks correctly** using `fs::symlink_metadata()` to avoid following broken symlinks
   - **Recursively processes directories** depth-first
   - **Parallelizes directory contents** using `par_bridge()` on directory entries
   - Returns count of items removed for user feedback
   - Supports dry-run mode (simulation without actual deletion)

### Concurrency Model

The tool uses **nested parallelism**:
- **Outer level**: Multiple input paths processed in parallel via `cli.paths.par_iter()`
- **Inner level**: When processing a directory, all child entries are removed in parallel via `par_bridge()`

This nested approach can lead to high thread utilization on large directory trees.

### Error Handling

- Uses `Result<u64, String>` to propagate errors and count items
- Errors during directory entry processing are logged but cause the entire operation to fail
- Distinguishes between non-existent paths and broken symlinks

## Key Dependencies

- **clap**: CLI argument parsing with derive macros
- **colored**: Terminal color output for user feedback
- **rayon**: Data parallelism for concurrent file operations
