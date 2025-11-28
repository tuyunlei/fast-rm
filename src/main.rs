use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::Parser;
use colored::*;
use rayon::prelude::*;

/// Errors that can occur during removal operations
#[derive(Debug)]
enum RemoveError {
    /// Failed to get metadata for a path
    MetadataFailed(PathBuf, io::Error),
    /// Failed to remove a file or symlink
    RemoveFailed(PathBuf, io::Error),
    /// Failed to read directory contents
    ReadDirFailed(PathBuf, io::Error),
    /// Failed to remove a directory
    RemoveDirFailed(PathBuf, io::Error),
    /// Failed to access a directory entry
    DirEntryFailed(PathBuf, io::Error),
    /// Path is not a recognized type (file, directory, or symlink)
    UnsupportedType(PathBuf),
    /// Path overlap detected (concurrent access issue)
    PathOverlap(String),
}

impl fmt::Display for RemoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoveError::MetadataFailed(path, err) => {
                write!(f, "Failed to get metadata for {:?}: {}", path, err)
            }
            RemoveError::RemoveFailed(path, err) => {
                write!(f, "Failed to remove {:?}: {}", path, err)
            }
            RemoveError::ReadDirFailed(path, err) => {
                write!(f, "Failed to read directory {:?}: {}", path, err)
            }
            RemoveError::RemoveDirFailed(path, err) => {
                write!(f, "Failed to remove directory {:?}: {}", path, err)
            }
            RemoveError::DirEntryFailed(path, err) => {
                write!(f, "Error accessing directory entry in {:?}: {}", path, err)
            }
            RemoveError::UnsupportedType(path) => {
                write!(
                    f,
                    "Path {:?} is not a file, directory, or symlink that can be removed",
                    path
                )
            }
            RemoveError::PathOverlap(msg) => write!(f, "{}", msg),
        }
    }
}

/// Configuration for removal operations
#[derive(Debug, Clone, Copy)]
struct RemoveConfig {
    verbose: bool,
    dry_run: bool,
    continue_on_error: bool,
}

impl RemoveConfig {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            verbose: cli.verbose,
            dry_run: cli.dry_run,
            continue_on_error: cli.continue_on_error,
        }
    }

    /// Log an action being performed on a path
    fn log_action(&self, action: &str, action_dry: &str, path: &Path, color: colored::Color) {
        if self.verbose || self.dry_run {
            let msg = if self.dry_run { action_dry } else { action };
            println!("  {}{:?}", msg.color(color), path);
        }
    }

    /// Log a checking action
    fn log_check(&self, path: &Path) {
        if self.verbose {
            let msg = if self.dry_run {
                "Would check "
            } else {
                "Checking "
            };
            println!("  {}{:?}", msg.dimmed(), path);
        }
    }
}

#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about = "A fast, concurrent file and directory remover."
)]
#[clap(long_about = None)] // Use `long_about` from `about`
struct Cli {
    /// Files or directories to remove
    #[clap(required = true, num_args = 1..)]
    paths: Vec<PathBuf>,

    /// Provide verbose output
    #[clap(short, long)]
    verbose: bool,

    /// Do not actually remove anything, just show what would be done
    #[clap(short = 'n', long = "dry-run")]
    dry_run: bool,

    /// Number of threads to use (defaults to number of CPU cores)
    #[clap(short = 'j', long = "threads")]
    threads: Option<usize>,

    /// Continue processing even if errors occur
    #[clap(short = 'c', long = "continue-on-error")]
    continue_on_error: bool,
}

/// Process results from removal operations and return statistics
fn process_results(
    results: Vec<(&PathBuf, Result<u64, RemoveError>)>,
    config: &RemoveConfig,
) -> (u64, u64) {
    let mut total_errors = 0;
    let mut total_items = 0;

    for (path, result) in results {
        match result {
            Ok(count) => {
                total_items += count;
                if count > 0 || config.verbose {
                    // Only print success if something was (or would be) done, or if verbose
                    println!(
                        "{} {:?} ({} {} {})",
                        if config.dry_run {
                            "Would successfully remove".green()
                        } else {
                            "Successfully removed".green()
                        },
                        path,
                        count,
                        if count == 1 { "item" } else { "items" },
                        if config.dry_run { "processed" } else { "deleted" }
                    );
                }
            }
            Err(e) => {
                total_errors += 1;
                eprintln!("{} {:?}: {}", "Failed to remove".red(), path, e.to_string().red());
            }
        }
    }

    (total_items, total_errors)
}

/// Print final summary and exit with appropriate code
fn print_summary_and_exit(total_items: u64, total_errors: u64, config: &RemoveConfig) -> ! {
    if config.dry_run {
        println!("{}", "Dry run finished.".yellow().bold());
    }

    if total_items > 0 || config.verbose {
        println!(
            "\n{} {} total {} {}.",
            "Summary:".bold(),
            total_items,
            if total_items == 1 { "item" } else { "items" },
            if config.dry_run {
                "would be removed"
            } else {
                "removed"
            }
        );
    }

    if total_errors > 0 {
        eprintln!(
            "{} {} error(s) encountered.",
            "Errors:".bold().red(),
            total_errors
        );
        std::process::exit(1);
    }

    std::process::exit(0);
}

/// Deduplicate and check for overlapping paths to prevent concurrent access issues
fn deduplicate_and_check_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, RemoveError> {
    let mut canonical_paths = Vec::new();
    let mut seen = HashSet::new();

    // First, canonicalize all paths
    for path in paths {
        match path.canonicalize() {
            Ok(canonical) => {
                if !seen.contains(&canonical) {
                    seen.insert(canonical.clone());
                    canonical_paths.push(canonical);
                }
            }
            Err(e) => {
                // If canonicalize fails, the path might not exist yet, or we don't have permission
                // In this case, we'll still try to use the original path
                eprintln!(
                    "{} Failed to canonicalize {:?}: {}. Using original path.",
                    "Warning:".yellow(),
                    path,
                    e
                );
                if !seen.contains(path) {
                    seen.insert(path.clone());
                    canonical_paths.push(path.clone());
                }
            }
        }
    }

    // Check for overlapping paths (one is ancestor of another)
    for i in 0..canonical_paths.len() {
        for j in (i + 1)..canonical_paths.len() {
            let path_i = &canonical_paths[i];
            let path_j = &canonical_paths[j];

            if path_i.starts_with(path_j) {
                return Err(RemoveError::PathOverlap(format!(
                    "Path overlap detected: {:?} is inside {:?}. This could cause concurrent access issues.",
                    path_i, path_j
                )));
            }
            if path_j.starts_with(path_i) {
                return Err(RemoveError::PathOverlap(format!(
                    "Path overlap detected: {:?} is inside {:?}. This could cause concurrent access issues.",
                    path_j, path_i
                )));
            }
        }
    }

    Ok(canonical_paths)
}

fn main() {
    let cli = Cli::parse();

    // Configure Rayon thread pool to prevent resource exhaustion from nested parallelism
    if let Some(num_threads) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()
            .expect("Failed to initialize thread pool");
    }

    // Deduplicate and check for overlapping paths to prevent concurrent access issues
    let paths_to_process = match deduplicate_and_check_paths(&cli.paths) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    let config = RemoveConfig::from_cli(&cli);

    if config.dry_run {
        println!(
            "{}",
            "Dry run mode activated. No files will be deleted."
                .yellow()
                .bold()
        );
    }

    let results: Vec<_> = paths_to_process
        .par_iter()
        .map(|path| {
            if config.verbose || config.dry_run {
                println!(
                    "{} {:?}...",
                    if config.dry_run {
                        "Would process".blue()
                    } else {
                        "Processing".cyan()
                    },
                    path
                );
            }
            let result = fast_remove(path, &config);
            (path, result)
        })
        .collect();

    let (total_items, total_errors) = process_results(results, &config);
    print_summary_and_exit(total_items, total_errors, &config);
}

/// Remove a symlink
fn remove_symlink(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    config.log_action(
        "Removing symlink ",
        "Would remove symlink ",
        path,
        colored::Color::Yellow,
    );
    if !config.dry_run {
        fs::remove_file(path)
            .map_err(|e| RemoveError::RemoveFailed(path.to_path_buf(), e))?;
    }
    Ok(1)
}

/// Remove a regular file
fn remove_file(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    config.log_action(
        "Removing file ",
        "Would remove file ",
        path,
        colored::Color::Yellow,
    );
    if !config.dry_run {
        fs::remove_file(path)
            .map_err(|e| RemoveError::RemoveFailed(path.to_path_buf(), e))?;
    }
    Ok(1)
}

/// Remove a directory and all its contents recursively
fn remove_directory(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    config.log_action(
        "Entering directory ",
        "Would enter directory ",
        path,
        colored::Color::Blue,
    );

    let children = fs::read_dir(path)
        .map_err(|e| RemoveError::ReadDirFailed(path.to_path_buf(), e))?;

    let results: Vec<Result<u64, RemoveError>> = children
        .par_bridge()
        .filter_map(|entry_result| match entry_result {
            Ok(entry) => Some(fast_remove(entry.path(), config)),
            Err(e) => {
                // Log and return error for problematic directory entries
                let error = RemoveError::DirEntryFailed(path.to_path_buf(), e);
                eprintln!("  {}", error.to_string().red().dimmed());
                Some(Err(error))
            }
        })
        .collect();

    // Separate successful and failed results
    let (successes, errors): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);

    // Sum up all successfully removed items
    let mut items_removed_count: u64 = successes
        .into_iter()
        .map(|r| r.unwrap()) // Safe because we partitioned by is_ok
        .sum();

    // Handle errors based on continue_on_error setting
    if !errors.is_empty() {
        if config.continue_on_error {
            eprintln!(
                "  {} {} error(s) in subdirectory {:?}, continuing...",
                "Warning:".yellow(),
                errors.len(),
                path
            );
        } else {
            // Return the first error
            return Err(errors.into_iter().next().unwrap().unwrap_err());
        }
    }

    config.log_action(
        "Removing empty directory ",
        "Would remove empty directory ",
        path,
        colored::Color::Yellow,
    );
    if !config.dry_run {
        fs::remove_dir(path)
            .map_err(|e| RemoveError::RemoveDirFailed(path.to_path_buf(), e))?;
    }
    items_removed_count += 1; // Count the directory itself
    Ok(items_removed_count)
}

/// Main entry point for removing a path (file, directory, or symlink)
fn fast_remove(path_ref: impl AsRef<Path>, config: &RemoveConfig) -> Result<u64, RemoveError> {
    let path = path_ref.as_ref();

    config.log_check(path);

    // Use symlink_metadata to correctly assess symlinks, even broken ones.
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| RemoveError::MetadataFailed(path.to_path_buf(), e))?;

    if metadata.file_type().is_symlink() {
        remove_symlink(path, config)
    } else if metadata.is_file() {
        remove_file(path, config)
    } else if metadata.is_dir() {
        remove_directory(path, config)
    } else {
        Err(RemoveError::UnsupportedType(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs as unix_fs;
    use tempfile::TempDir;

    /// 🛡️ SAFETY GUARD: Validate that path is within allowed test directory
    /// This prevents accidental deletion of files outside the test sandbox
    fn validate_test_path(path: &Path, allowed_root: &Path) -> Result<(), String> {
        // Canonicalize both paths to resolve symlinks and relative paths
        let canonical_path = if path.exists() {
            path.canonicalize()
                .map_err(|e| format!("Cannot canonicalize path {:?}: {}", path, e))?
        } else {
            // If path doesn't exist yet, canonicalize parent instead
            let parent = path
                .parent()
                .ok_or_else(|| "No parent directory".to_string())?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| format!("Cannot canonicalize parent {:?}: {}", parent, e))?;
            canonical_parent.join(path.file_name().unwrap())
        };

        let canonical_root = allowed_root
            .canonicalize()
            .map_err(|e| format!("Cannot canonicalize root {:?}: {}", allowed_root, e))?;

        if !canonical_path.starts_with(&canonical_root) {
            panic!(
                "🚨 SAFETY VIOLATION: Path {:?} is outside allowed test directory {:?}",
                canonical_path, canonical_root
            );
        }
        Ok(())
    }

    // ===== STAGE 1: Pure Logic Tests (No File Operations) =====

    #[test]
    fn test_remove_config_from_cli() {
        let cli = Cli {
            paths: vec![PathBuf::from("/tmp/test")],
            verbose: true,
            dry_run: false,
            threads: None,
            continue_on_error: true,
        };

        let config = RemoveConfig::from_cli(&cli);
        assert_eq!(config.verbose, true);
        assert_eq!(config.dry_run, false);
        assert_eq!(config.continue_on_error, true);
    }

    #[test]
    fn test_path_deduplication() {
        let temp_dir = TempDir::new().unwrap();
        let path1 = temp_dir.path().join("file.txt");
        let path2 = temp_dir.path().join("file.txt"); // duplicate

        // Create the file so canonicalize works
        File::create(&path1).unwrap();

        let paths = vec![path1.clone(), path2.clone()];
        let result = deduplicate_and_check_paths(&paths).unwrap();

        // Should deduplicate to 1 path
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_path_overlap_detection() {
        let temp_dir = TempDir::new().unwrap();
        let parent = temp_dir.path().join("parent");
        let child = parent.join("child");

        // Create both directories
        std::fs::create_dir_all(&child).unwrap();

        let paths = vec![parent.clone(), child.clone()];
        let result = deduplicate_and_check_paths(&paths);

        // Should detect overlap and return error
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, RemoveError::PathOverlap(_)));
    }

    #[test]
    fn test_remove_error_display() {
        let path = PathBuf::from("/tmp/test");
        let error = RemoveError::UnsupportedType(path.clone());
        let display = format!("{}", error);
        assert!(display.contains("/tmp/test"));
        assert!(display.contains("not a file, directory, or symlink"));
    }

    // ===== STAGE 2: Dry-Run Tests (No Actual Deletion) =====

    #[test]
    fn test_dry_run_does_not_delete() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        // Create test file
        std::fs::write(&test_file, "test content").unwrap();
        assert!(test_file.exists());

        // Run with dry_run = true
        let config = RemoveConfig {
            verbose: false,
            dry_run: true,
            continue_on_error: false,
        };

        let result = remove_file(&test_file, &config);
        assert!(result.is_ok());

        // File should still exist
        assert!(test_file.exists(), "Dry-run should not delete files");
    }

    #[test]
    fn test_dry_run_directory() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("testdir");
        std::fs::create_dir(&test_dir).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: true,
            continue_on_error: false,
        };

        let result = fast_remove(&test_dir, &config);
        assert!(result.is_ok());

        // Directory should still exist
        assert!(test_dir.exists(), "Dry-run should not delete directories");
    }

    // ===== STAGE 3: Real Deletion Tests (With Path Guard) =====
    // These tests use #[ignore] and require explicit opt-in
    // Run with: cargo test -- --ignored

    #[test]
    #[ignore = "Performs real file deletion - run explicitly with --ignored"]
    fn test_remove_single_file_guarded() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        // Create test file
        std::fs::write(&test_file, "test content").unwrap();
        assert!(test_file.exists());

        // 🛡️ Safety check
        validate_test_path(&test_file, temp_dir.path()).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: false,
            continue_on_error: false,
        };

        let result = remove_file(&test_file, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        // File should be deleted
        assert!(!test_file.exists());
    }

    #[test]
    #[ignore = "Performs real file deletion - run explicitly with --ignored"]
    fn test_remove_empty_directory_guarded() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("empty_dir");
        std::fs::create_dir(&test_dir).unwrap();

        // 🛡️ Safety check
        validate_test_path(&test_dir, temp_dir.path()).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: false,
            continue_on_error: false,
        };

        let result = fast_remove(&test_dir, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
        assert!(!test_dir.exists());
    }

    #[test]
    #[ignore = "Performs real file deletion - run explicitly with --ignored"]
    fn test_remove_nested_directory_guarded() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("nested");
        let sub_dir = test_dir.join("subdir");
        let file1 = test_dir.join("file1.txt");
        let file2 = sub_dir.join("file2.txt");

        // Create structure
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(&file1, "content1").unwrap();
        std::fs::write(&file2, "content2").unwrap();

        // 🛡️ Safety checks
        validate_test_path(&test_dir, temp_dir.path()).unwrap();
        validate_test_path(&file1, temp_dir.path()).unwrap();
        validate_test_path(&file2, temp_dir.path()).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: false,
            continue_on_error: false,
        };

        let result = fast_remove(&test_dir, &config);
        assert!(result.is_ok());
        // Should remove: file1, file2, subdir, nested = 4 items
        assert_eq!(result.unwrap(), 4);
        assert!(!test_dir.exists());
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "Performs real file deletion - run explicitly with --ignored"]
    fn test_remove_symlink_guarded() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target.txt");
        let link = temp_dir.path().join("link.txt");

        // Create target and symlink
        std::fs::write(&target, "target content").unwrap();
        unix_fs::symlink(&target, &link).unwrap();
        assert!(link.exists());

        // 🛡️ Safety check
        validate_test_path(&link, temp_dir.path()).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: false,
            continue_on_error: false,
        };

        let result = remove_symlink(&link, &config);
        assert!(result.is_ok());
        assert!(!link.exists());
        assert!(target.exists(), "Target should not be deleted");
    }

    #[test]
    #[ignore = "Performs real file deletion - run explicitly with --ignored"]
    fn test_continue_on_error_guarded() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("test");
        let file1 = test_dir.join("file1.txt");
        let file2 = test_dir.join("file2.txt");

        std::fs::create_dir(&test_dir).unwrap();
        std::fs::write(&file1, "content1").unwrap();
        std::fs::write(&file2, "content2").unwrap();

        // Make file1 read-only to cause error (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file1.metadata().unwrap().permissions();
            perms.set_mode(0o000);
            std::fs::set_permissions(&file1, perms).unwrap();
        }

        // 🛡️ Safety check
        validate_test_path(&test_dir, temp_dir.path()).unwrap();

        let config = RemoveConfig {
            verbose: false,
            dry_run: false,
            continue_on_error: true, // Continue despite errors
        };

        let result = fast_remove(&test_dir, &config);

        // Should succeed with continue_on_error
        // (Though some items might fail to delete due to permissions)
        assert!(result.is_ok());
    }
}
