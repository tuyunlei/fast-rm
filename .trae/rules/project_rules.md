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
cargo run -- <paths>                    # Run with paths to remove
cargo run -- --help                     # Show help
cargo run -- -v <paths>                 # Standard verbosity (shows 10 recent files)
cargo run -- -vv <paths>                # Detailed verbosity (shows terminal-height files)
cargo run -- -n <paths>                 # Dry run (show what would be deleted)
cargo run -- -j 8 <paths>               # Use 8 threads (default: CPU cores)
cargo run -- -c <paths>                 # Continue on error (don't stop on failures)
cargo run -- -v -n -c <paths>           # Combine flags
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
   - CLI flags: `-v/--verbose` (multiple levels), `-n/--dry-run`, `-j/--threads`, `-c/--continue-on-error`
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
   - `RemoveProgress`: Thread-safe progress counters using `AtomicUsize`
   - `ProgressDisplay`: Renders live TUI with `indicatif` and `crossterm`
   - Tracks: scanned items, deleted items, errors, recent files, deletion speed
   - Verbosity-aware display (Simple: summary only, Standard: 10 files, Detailed: terminal-height lines)

6. **`src/removal.rs`** - Core deletion logic
   - `fast_remove()`: Main recursive removal function
   - `remove_file()`, `remove_symlink()`, `remove_directory()`: Type-specific handlers
   - Uses `fs::symlink_metadata()` to correctly handle symlinks without following them
   - **Parallelizes directory contents** using `par_bridge()` from rayon

7. **`src/results.rs`** - Result processing and output formatting
   - `process_results()`: Aggregates results from parallel operations
   - `print_summary_and_exit()`: Final summary and exit code handling

8. **`src/main.rs`** - Entry point and orchestration
   - Initializes thread pool (configurable via `-j` flag)
   - Spawns background TUI thread for live progress updates
   - Coordinates parallel path processing using rayon's `par_iter()`
   - Synchronizes TUI shutdown with completion

### Concurrency Model

The tool uses **nested parallelism**:
- **Outer level**: Multiple input paths processed in parallel via `paths.par_iter()`
- **Inner level**: When processing a directory, all child entries are removed in parallel via `par_bridge()`
- **Thread pool**: Configurable via `-j/--threads` flag (defaults to number of CPU cores)
- **Live TUI**: Separate background thread updates progress display every 50ms using atomic operations

### Safety Features

1. **Path overlap detection** (`src/path.rs`): Prevents concurrent deletion of parent and child directories
2. **Symlink handling**: Uses `symlink_metadata()` to avoid following broken symlinks
3. **Dry-run mode**: Simulates deletions without actual file operations
4. **Continue-on-error**: Optional flag to keep processing after encountering errors

### Progress Tracking Architecture

- `RemoveProgress` uses atomic counters for lock-free updates from parallel workers
- Recent files and errors stored in `Mutex<VecDeque>` with bounded capacity (50 items)
- TUI thread polls progress atomics without blocking deletion workers
- Final synchronization ensures TUI displays complete results before exit

## Key Dependencies

- **clap**: CLI argument parsing with derive macros
- **colored**: Terminal color output for user feedback
- **rayon**: Data parallelism for concurrent file operations
- **indicatif**: Progress bars and TUI rendering
- **crossterm**: Terminal size detection for adaptive display
- **tempfile** (dev): Temporary directories for testing

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
