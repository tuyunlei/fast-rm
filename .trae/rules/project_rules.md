# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`fast-rm` is a high-performance, concurrent file and directory removal tool written in Rust. It uses a two-pool architecture with separate scanner and deleter thread pools, coordinated via a lock-free work queue, to maximize deletion throughput and provide accurate real-time progress tracking.

## Development Commands

### Building
```bash
cargo build          # Debug build
cargo build --release  # Optimized release build
```

### Running
```bash
cargo run -- <paths>                        # Run with paths to remove
cargo run -- --help                         # Show help
cargo run -- -v <paths>                     # Standard verbosity (shows 10 recent files)
cargo run -- -vv <paths>                    # Detailed verbosity (shows terminal-height files)
cargo run -- -n <paths>                     # Dry run (show what would be deleted)
cargo run -- -j 8 <paths>                   # Use 8 threads for both pools (default: CPU cores)
cargo run -- --scan-threads 4 <paths>       # Use 4 scanner threads
cargo run -- --delete-threads 8 <paths>     # Use 8 deleter threads
cargo run -- --scan-threads 4 --delete-threads 8 <paths>  # Independent pool sizing
cargo run -- -c <paths>                     # Continue on error (don't stop on failures)
cargo run -- -v -n -c <paths>               # Combine flags
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

### Adding Dependencies
```bash
cargo add <crate>              # Add latest version of a crate
cargo add <crate>@<version>    # Add specific version
```

**IMPORTANT**: Always prefer `cargo add` over manually editing `Cargo.toml` to ensure you get the latest compatible versions and correct feature flags.

## Architecture

### Module Structure

The application has been refactored into a modular architecture with clear separation of concerns:

1. **`src/cli.rs`** - Command-line interface definition
   - Defines `Cli` struct with `clap` derive macros
   - CLI flags: `-v/--verbose` (multiple levels), `-n/--dry-run`, `-j/--threads`, `--scan-threads`, `--delete-threads`, `-c/--continue-on-error`
   - Helper methods: `get_scan_threads()`, `get_delete_threads()` with fallback logic
   - Accepts multiple paths as required arguments

2. **`src/errors.rs`** - Custom error types
   - `RemoveError` enum with variants for different failure modes
   - Wraps `io::Error` with path context for better error messages
   - Includes: `MetadataFailed`, `RemoveFailed`, `ReadDirFailed`, `RemoveDirFailed`, `DirEntryFailed`, `UnsupportedType`, `PathOverlap`

3. **`src/config.rs`** - Configuration and verbosity management
   - `Verbosity` enum: `Simple` (default), `Standard` (-v), `Detailed` (-vv)
   - `RemoveConfig` struct that aggregates all runtime options
   - Helper methods for logging actions at different verbosity levels

4. **`src/path.rs`** - Path validation and deduplication
   - `deduplicate_and_check_paths()`: Canonicalizes paths, removes duplicates, detects overlaps
   - **Safety check**: Prevents concurrent deletion of nested paths (parent/child conflict detection)

5. **`src/progress.rs`** - TUI progress tracking
   - `RemoveProgress`: Thread-safe progress counters using `AtomicUsize` with cache line padding
   - Lock-free channels (`crossbeam_channel`) for recent files and errors (replaces Mutex<VecDeque>)
   - `ProgressDisplay`: Renders live TUI with `indicatif` and `crossterm`
   - Tracks: scanned items, deleted items, errors, queue depth, deletion speed
   - Verbosity-aware display (Simple: summary only, Standard: 10 files, Detailed: terminal-height lines)
   - **Performance**: Channels use `try_send()` for non-blocking updates, TUI-local VecDeque cache eliminates allocations

6. **`src/queue.rs`** - Work queue for scan/delete coordination
   - `FileJob` enum: File, Symlink, EmptyDir (directories enqueued AFTER children)
   - `AdaptiveQueue`: Bounded MPMC channel wrapper with depth tracking
   - Coordinating layer between scanner and deleter thread pools
   - Queue capacity: `scan_threads * 1000`, min 10,000 items

7. **`src/scanner.rs`** - Parallel directory scanning
   - `scan_path()`: Recursively traverses directory trees, enqueues FileJob items
   - `scan_directory()`: Uses rayon's `par_bridge()` for parallel child processing
   - **Depth-first traversal**: Ensures directories enqueued after all children (critical for deletion order)
   - Increments `progress.scanned` counter, handles errors with `continue_on_error`

8. **`src/deleter.rs`** - Concurrent deletion workers
   - `delete_worker()`: Consumer loop that processes FileJob items from queue
   - Type-specific handlers: `delete_file()`, `delete_symlink()`, `delete_empty_dir()`
   - **Shutdown logic**: Exits when `scanners_done` AND queue empty
   - Uses `recv_timeout()` with 100ms intervals to check completion status
   - No recursion needed (scanner already enqueued everything)

9. **`src/removal.rs`** - Legacy single-pool deletion logic *(deprecated)*
   - `fast_remove()`: Recursive removal function (used by old architecture)
   - `remove_file()`, `remove_symlink()`, `remove_directory()`: Type-specific handlers
   - **Note**: This module is retained for compatibility but not used by the two-pool architecture

10. **`src/results.rs`** - Result processing and output formatting
    - `print_summary_and_exit()`: Final summary and exit code handling
    - Simplified from old architecture (no longer aggregates results from `par_iter()`)

11. **`src/main.rs`** - Entry point and two-pool orchestration
    - Creates `AdaptiveQueue` for scan/delete coordination
    - Spawns scanner thread pool (rayon with custom pool, named threads)
    - Spawns deleter worker threads (N threads running `delete_worker()` loop)
    - Spawns TUI thread with queue depth tracking
    - **Coordination**: AtomicBool signals scanner completion, deleters drain queue, TUI updates every 50ms
    - Final stats collected directly from `RemoveProgress` atomic counters

### Concurrency Model

The tool uses a **two-pool architecture** with complete separation between scanning and deletion:

#### Scanner Thread Pool (Rayon-based)
- Custom rayon thread pool created with `ThreadPoolBuilder`
- Pool size: `cli.get_scan_threads()` (default: CPU cores)
- **Parallel scanning**: Multiple paths scanned concurrently via `par_iter()`
- **Parallel directory traversal**: Child entries scanned via `par_bridge()` within each directory
- **Work enqueuing**: Scanners enqueue `FileJob` items into the `AdaptiveQueue`
- **Completion signal**: Sets `scanners_done` AtomicBool when all scanning complete

#### Deleter Thread Pool (Worker threads)
- Pool size: `cli.get_delete_threads()` (default: CPU cores)
- Each thread runs `delete_worker()` in a loop
- **Work consumption**: Dequeues `FileJob` items from `AdaptiveQueue` using `recv_timeout(100ms)`
- **Concurrent deletion**: Multiple deleters process different items simultaneously
- **Shutdown logic**: Exits when `scanners_done` is true AND queue is empty

#### Work Queue Coordination
- `AdaptiveQueue`: Bounded MPMC channel (capacity: `scan_threads * 1000`, min 10,000)
- **FileJob types**: File, Symlink, EmptyDir (order preserves parent-after-children)
- **Backpressure**: Scanners block on `send()` if queue full (prevents memory explosion)
- **Depth tracking**: `queue.depth()` = enqueued - dequeued (lock-free atomic counters)

#### TUI Thread
- Separate background thread updates progress display every 50ms
- Reads atomic counters: `scanned`, `deleted`, `errors`, `queue.depth()`
- **Lock-free**: All reads use `Ordering::Relaxed`, no blocking of workers
- Display format: `"{scanned} scanned | {queue_depth} in queue | {deleted} deleted | {errors} errors | {speed} items/s"`

#### Benefits of Two-Pool Design
1. **Accurate progress**: Scanning completes first, giving exact total count
2. **Maximum throughput**: Scanners don't wait for deleters, deleters don't wait for scanners
3. **Independent tuning**: Adjust scan vs delete parallelism independently
4. **Better resource utilization**: CPU-bound scanning and I/O-bound deletion can overlap
5. **Queue visibility**: Real-time queue depth shows pipeline health

### Safety Features

1. **Path overlap detection** (`src/path.rs`): Prevents concurrent deletion of parent and child directories
2. **Symlink handling**: Uses `symlink_metadata()` to avoid following broken symlinks
3. **Dry-run mode**: Simulates deletions without actual file operations
4. **Continue-on-error**: Optional flag to keep processing after encountering errors

### Progress Tracking Architecture

#### RemoveProgress (Lock-Free Design)
- **Atomic counters**: `scanned`, `deleted`, `errors` (AtomicUsize with 64-byte cache line padding)
- **Channels**: `crossbeam_channel` bounded channels for recent files/errors (capacity: 1000/100)
- **Non-blocking updates**: Workers use `try_send()`, drops if channel full (acceptable for display)
- **Memory efficiency**: Arc<Path> instead of PathBuf cloning (1 allocation vs 2-4 clones per file)

#### ProgressDisplay (TUI Rendering)
- **TUI-local cache**: Mutex<VecDeque> in ProgressDisplay (not shared with workers)
- **Update loop**: Drains channels into cache incrementally, no Vec allocation per update
- **Queue depth**: Passed as `Option<usize>` to `update()` and `finish()`
- **Display modes**:
  - Legacy: Shows deleted/scanned without queue depth
  - Two-pool: Shows `scanned | in_queue | deleted | errors | speed`

#### Performance Optimizations
1. **Cache line padding**: Prevents false sharing between atomic counters on multi-core systems
2. **Lock-free channels**: Eliminates 16,000+ mutex acquisitions/sec from old architecture
3. **Arc<Path> sharing**: Reduces allocations from 2-4 clones to 1 Arc per file
4. **Non-blocking sends**: `try_send()` never blocks workers, gracefully degrades display
5. **Incremental cache updates**: TUI drains channels into local cache, avoids repeated allocations

## Key Dependencies

- **clap**: CLI argument parsing with derive macros
- **colored**: Terminal color output for user feedback
- **rayon**: Data parallelism for scanner thread pool (custom pool creation)
- **crossbeam-channel**: Lock-free MPMC channels for work queue and progress tracking
- **num_cpus**: CPU core detection for default thread counts
- **indicatif**: Progress bars and TUI rendering
- **crossterm**: Terminal size detection for adaptive display
- **tempfile** (dev): Temporary directories for testing

## Testing

The project has comprehensive test coverage across three layers:

### Unit Tests (15 tests in src/)
- **errors.rs**: Error type display formatting
- **path.rs**: Path deduplication and overlap detection (safety-critical)
- **queue.rs**: AdaptiveQueue send/recv, depth tracking, EmptyDir variant
- **scanner.rs**: Single file, directory with files, nested directories
- **deleter.rs**: File deletion, dry-run mode, empty dir, worker loop
- **removal.rs**: Legacy dry-run tests (deprecated module)

### Integration Tests (7 tests in tests/concurrency_tests.rs)
**Purpose**: Validate concurrency safety of two-pool architecture

1. **test_concurrent_scan_delete_no_data_races**
   - 10 directories × 50 files = 500 files total
   - Validates test setup integrity

2. **test_no_items_lost_in_concurrent_processing**
   - Ensures scanned set equals deleted set (no lost items)
   - Tracks all items through concurrent processing

3. **test_atomic_counter_accuracy_under_contention**
   - 10 threads × 1,000 increments = 10,000 operations
   - Validates AtomicUsize remains accurate under high contention

4. **test_scanner_deleter_coordination**
   - Simulates 100 items scanned/deleted with delays
   - Verifies deleters wait for scanners AND queue drain
   - Tests Release/Acquire memory ordering

5. **test_multiple_paths_concurrent_processing**
   - 5 independent directory trees in parallel
   - Ensures no conflicts between concurrent paths

6. **test_error_handling_doesnt_deadlock**
   - Permission errors with restricted files
   - Verifies error paths don't cause hangs

7. **test_queue_ordering_preserves_parent_child_relationship** *(CRITICAL)*
   - Validates file → dir3 → dir2 → dir1 deletion order
   - Ensures directories always deleted AFTER children
   - Prevents "directory not empty" errors

### Running Tests
```bash
cargo test                        # All 22 tests (15 unit + 7 concurrency)
cargo test --test concurrency_tests  # Just concurrency tests
cargo test test_queue             # Specific test by name
```

## Development Workflow

### Branch Model

This project follows a simplified **GitHub Flow** branching strategy:

- **`main`** - The primary branch, always stable and deployable
- **Feature branches** - Short-lived branches for developing features, fixes, or refactoring
  - Naming convention: `feature/<description>`, `fix/<description>`, `refactor/<description>`
  - Examples: `feature/add-progress-bar`, `fix/symlink-handling`, `refactor/split-modules`

**Branch Workflow:**
1. Create branch from `main`
2. Make incremental commits with quality checks
3. Merge back to `main` using `--no-ff` (preserves branch history)
4. Delete feature branch after merge

### Commit Message Convention

Follow **Conventional Commits** specification for clear, semantic commit history:

**Format:**
```
<type>(<scope>): <subject>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature or enhancement
- `fix`: Bug fix
- `refactor`: Code restructuring without behavior change
- `perf`: Performance improvement
- `test`: Adding or updating tests
- `docs`: Documentation changes (including CLAUDE.md)
- `chore`: Build process, dependencies, tooling
- `style`: Code formatting (whitespace, semicolons, etc.)

**Scope (optional):** Module or component affected (e.g., `cli`, `progress`, `removal`, `errors`)

**Examples:**
```
feat(progress): add TUI with live file display

Implemented real-time progress tracking using indicatif.
Shows deletion speed, recent files, and error count.

feat: add --threads flag for thread pool configuration

fix(removal): handle broken symlinks correctly

Use symlink_metadata instead of metadata to avoid
following broken symlinks during deletion.

refactor: split main.rs into modular components

Extracted cli, errors, config, progress, path, removal,
and results modules for better maintainability.

docs: update CLAUDE.md with new module structure

test(path): add overlap detection test cases

chore: bump rayon to 1.10.0
```

**Subject Guidelines:**
- Use imperative mood ("add" not "added" or "adds")
- No capitalization of first letter
- No period at the end
- Maximum 72 characters
- Clearly describe what the commit does

**Body Guidelines (optional but recommended for complex changes):**
- Explain the motivation for the change
- Describe what was changed and why
- Wrap at 72 characters

### Working on Long-Running Tasks

For complex features or significant refactoring that require multiple steps:

1. **Create a Feature Branch**
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Break Work into Fine-Grained Commits**
   - Each commit should represent a single, logical change
   - Follow the commit convention above
   - Each commit MUST pass all quality checks before committing:
     ```bash
     cargo fmt                # Format code
     cargo check              # Quick compile check
     cargo clippy             # Lint check
     cargo test               # Run all tests
     cargo build              # Full build verification
     ```
   - Only commit if all checks pass successfully

3. **Merge Back to Main**
   - Before merging, verify that CLAUDE.md is up-to-date:
     - Check if architecture descriptions match the current code
     - Update module descriptions if refactored
     - Add any new commands or workflows introduced
   - Merge using `--no-ff` to preserve branch history:
     ```bash
     git checkout main
     git merge --no-ff feature/your-feature-name
     ```
   - Delete the feature branch after successful merge:
     ```bash
     git branch -d feature/your-feature-name
     ```

**Rationale**: Fine-grained commits with quality checks ensure:
- Easy to identify which commit introduced an issue
- Each point in history is buildable and testable
- Clear development progression for future reference
- Branch history provides context for complex changes
- Semantic commit messages enable automated changelog generation
