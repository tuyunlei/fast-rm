use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use colored::*;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;

/// Verbosity level for output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verbosity {
    /// Simple mode: only progress bar and statistics
    Simple,
    /// Standard mode (-v): progress bar + recent 10 files
    Standard,
    /// Detailed mode (-vv): progress bar + terminal-adaptive file list
    Detailed,
}

impl Verbosity {
    fn from_count(count: u8) -> Self {
        match count {
            0 => Self::Simple,
            1 => Self::Standard,
            _ => Self::Detailed, // 2 or more
        }
    }

    fn is_verbose(&self) -> bool {
        matches!(self, Self::Standard | Self::Detailed)
    }
}

/// Progress tracker for removal operations
/// This abstraction supports both current edge-scan-edge-delete and future parallel scan/delete
#[derive(Debug)]
struct RemoveProgress {
    /// Number of items scanned
    scanned: AtomicUsize,
    /// Number of items deleted
    deleted: AtomicUsize,
    /// Number of errors encountered
    errors: AtomicUsize,
    /// Recent files (bounded queue for display)
    recent_files: Mutex<VecDeque<PathBuf>>,
    /// Error files (bounded queue for error display)
    error_files: Mutex<VecDeque<(PathBuf, String)>>,
    /// Start time for speed and ETA calculation
    start_time: Instant,
}

impl RemoveProgress {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            scanned: AtomicUsize::new(0),
            deleted: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            recent_files: Mutex::new(VecDeque::new()),
            error_files: Mutex::new(VecDeque::new()),
            start_time: Instant::now(),
        })
    }

    fn inc_scanned(&self) {
        self.scanned.fetch_add(1, Ordering::Relaxed);
    }

    fn inc_deleted(&self, path: PathBuf) {
        self.deleted.fetch_add(1, Ordering::Relaxed);

        // Add to recent files
        let mut recent = self.recent_files.lock().unwrap();
        recent.push_back(path);
        // Keep a buffer larger than what we might display, e.g., 50
        while recent.len() > 50 {
            recent.pop_front();
        }
    }

    fn inc_error(&self, path: PathBuf, error: String) {
        self.errors.fetch_add(1, Ordering::Relaxed);

        // Add to error files
        let mut errors = self.error_files.lock().unwrap();
        errors.push_back((path, error));
        // Keep a buffer larger than what we might display
        while errors.len() > 50 {
            errors.pop_front();
        }
    }

    fn get_stats(&self) -> (usize, usize, usize, f64, f64) {
        let scanned = self.scanned.load(Ordering::Relaxed);
        let deleted = self.deleted.load(Ordering::Relaxed);
        let errors = self.errors.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            deleted as f64 / elapsed
        } else {
            0.0
        };

        // Calculate ETA (estimated time remaining)
        let eta = if speed > 0.0 && scanned > deleted {
            (scanned - deleted) as f64 / speed
        } else {
            0.0
        };

        (scanned, deleted, errors, speed, eta)
    }

    fn get_recent_files(&self) -> Vec<PathBuf> {
        self.recent_files
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }

    fn get_error_files(&self) -> Vec<(PathBuf, String)> {
        self.error_files
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }
}

/// TUI display manager for progress visualization
struct ProgressDisplay {
    multi: MultiProgress,
    main_bar: ProgressBar,
    file_bars: Vec<ProgressBar>,
    error_bar: Option<ProgressBar>,
    verbosity: Verbosity,
}

impl ProgressDisplay {
    fn new(verbosity: Verbosity, dry_run: bool) -> Self {
        let multi = MultiProgress::new();

        // Create main progress bar
        let main_bar = multi.add(ProgressBar::new_spinner());
        let template = if dry_run {
            "[Dry Run] Scanned: {msg}"
        } else {
            "Deleted: {msg}"
        };
        main_bar.set_style(
            ProgressStyle::default_spinner()
                .template(template)
                .unwrap()
        );

        let mut file_bars = Vec::new();
        let mut error_bar = None;

        // Create file list bars based on verbosity
        match verbosity {
            Verbosity::Simple => {
                // No file bars in simple mode
            }
            Verbosity::Standard => {
                // 10 file bars for standard mode
                for _ in 0..10 {
                    let bar = multi.add(ProgressBar::new_spinner());
                    bar.set_style(
                        ProgressStyle::default_spinner()
                            .template("  {msg}")
                            .unwrap()
                    );
                    file_bars.push(bar);
                }
            }
            Verbosity::Detailed => {
                // Terminal height adaptive - get terminal height
                let height = crossterm::terminal::size()
                    .map(|(_, h)| h as usize)
                    .unwrap_or(24);

                // Reserve 5 lines for main bar, stats, and errors
                let file_count = (height.saturating_sub(5)).min(50).max(5);

                for _ in 0..file_count {
                    let bar = multi.add(ProgressBar::new_spinner());
                    bar.set_style(
                        ProgressStyle::default_spinner()
                            .template("  {msg}")
                            .unwrap()
                    );
                    file_bars.push(bar);
                }
            }
        }

        // Always create error bar
        let err_bar = multi.add(ProgressBar::new_spinner());
        err_bar.set_style(
            ProgressStyle::default_spinner()
                .template("{msg}")
                .unwrap()
        );
        error_bar = Some(err_bar);

        Self {
            multi,
            main_bar,
            file_bars,
            error_bar,
            verbosity,
        }
    }

    fn update(&self, progress: &RemoveProgress, dry_run: bool) {
        let (scanned, deleted, errors, speed, _eta) = progress.get_stats();

        // Update main bar
        let main_msg = if dry_run {
            format!("{} scanned | {} errors | {:.1} items/s",
                scanned, errors, speed)
        } else {
            format!("{} deleted | {} errors | {:.1} items/s",
                deleted, errors, speed)
        };
        self.main_bar.set_message(main_msg);

        // Update file bars
        if !self.file_bars.is_empty() {
            let recent_files = progress.get_recent_files();
            let display_count = self.file_bars.len().min(recent_files.len());

            // Update visible bars
            for (i, bar) in self.file_bars.iter().enumerate() {
                if i < display_count {
                    let file = &recent_files[recent_files.len() - display_count + i];
                    bar.set_message(format!("{:?}", file));
                } else {
                    bar.set_message("");
                }
            }
        }

        // Update error bar
        if let Some(err_bar) = &self.error_bar {
            if errors > 0 {
                let error_files = progress.get_error_files();
                if let Some((path, msg)) = error_files.last() {
                    err_bar.set_message(format!("Last error: {:?} - {}", path, msg));
                }
            } else {
                err_bar.set_message("");
            }
        }
    }

    fn finish(&self, progress: &RemoveProgress, dry_run: bool) {
        let (scanned, deleted, errors, _, _) = progress.get_stats();

        let final_msg = if dry_run {
            format!("✓ Dry run complete: {} items scanned, {} errors", scanned, errors)
        } else {
            format!("✓ Complete: {} items deleted, {} errors", deleted, errors)
        };

        self.main_bar.finish_with_message(final_msg);

        // Clear file bars
        for bar in &self.file_bars {
            bar.finish_and_clear();
        }

        // Keep error bar if there were errors
        if errors == 0 {
            if let Some(err_bar) = &self.error_bar {
                err_bar.finish_and_clear();
            }
        }
    }
}

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
#[derive(Debug, Clone)]
struct RemoveConfig {
    verbosity: Verbosity,
    dry_run: bool,
    continue_on_error: bool,
    progress: Option<Arc<RemoveProgress>>,
}

impl RemoveConfig {
    fn from_cli(cli: &Cli, progress: Option<Arc<RemoveProgress>>) -> Self {
        Self {
            verbosity: Verbosity::from_count(cli.verbosity),
            dry_run: cli.dry_run,
            continue_on_error: cli.continue_on_error,
            progress,
        }
    }

    /// Log an action being performed on a path
    fn log_action(&self, action: &str, action_dry: &str, path: &Path, color: colored::Color) {
        if self.verbosity.is_verbose() || self.dry_run {
            let msg = if self.dry_run { action_dry } else { action };
            println!("  {}{:?}", msg.color(color), path);
        }
    }

    /// Log a checking action
    fn log_check(&self, path: &Path) {
        if self.verbosity.is_verbose() {
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

    /// Verbosity level: -v for standard, -vv for detailed
    #[clap(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbosity: u8,

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
                if (count > 0 || config.verbosity.is_verbose()) && config.progress.is_none() {
                    // Only print success if something was (or would be) done, or if verbose
                    // AND if we are not using the TUI (progress is None)
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

    if total_items > 0 || config.verbosity.is_verbose() {
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

    // Create progress tracker and display
    let progress = RemoveProgress::new();
    let verbosity = Verbosity::from_count(cli.verbosity);
    let display = Arc::new(ProgressDisplay::new(verbosity, cli.dry_run));

    let config = RemoveConfig::from_cli(&cli, Some(progress.clone()));

    if config.dry_run {
        println!(
            "{}",
            "Dry run mode activated. No files will be deleted."
                .yellow()
                .bold()
        );
        println!(); // Add a newline before TUI starts
    }

    // Start TUI update thread
    let display_clone = display.clone();
    let progress_clone = progress.clone();
    let dry_run = cli.dry_run;
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_clone = is_done.clone();

    let tui_thread = thread::spawn(move || {
        while !is_done_clone.load(Ordering::Relaxed) {
            display_clone.update(&progress_clone, dry_run);
            thread::sleep(Duration::from_millis(50));
        }
        // Ensure final state is reflected
        display_clone.update(&progress_clone, dry_run);
    });

    let results: Vec<_> = paths_to_process
        .par_iter()
        .map(|path| {
            if config.progress.is_none() && (config.verbosity.is_verbose() || config.dry_run) {
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

    // Stop TUI thread
    is_done.store(true, Ordering::Relaxed);
    tui_thread.join().expect("TUI thread panicked");
    display.finish(&progress, cli.dry_run);

    let (total_items, total_errors) = process_results(results, &config);
    print_summary_and_exit(total_items, total_errors, &config);
}

/// Remove a symlink
fn remove_symlink(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Removing symlink ",
            "Would remove symlink ",
            path,
            colored::Color::Yellow,
        );
    }

    if !config.dry_run {
        match fs::remove_file(path) {
            Ok(_) => {
                if let Some(p) = &config.progress {
                    p.inc_deleted(path.to_path_buf());
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path.to_path_buf(), err_msg);
                }
                return Err(RemoveError::RemoveFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path.to_path_buf());
        }
    }
    Ok(1)
}

/// Remove a regular file
fn remove_file(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Removing file ",
            "Would remove file ",
            path,
            colored::Color::Yellow,
        );
    }

    if !config.dry_run {
        match fs::remove_file(path) {
            Ok(_) => {
                if let Some(p) = &config.progress {
                    p.inc_deleted(path.to_path_buf());
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path.to_path_buf(), err_msg);
                }
                return Err(RemoveError::RemoveFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path.to_path_buf());
        }
    }
    Ok(1)
}

/// Remove a directory and all its contents recursively
fn remove_directory(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Entering directory ",
            "Would enter directory ",
            path,
            colored::Color::Blue,
        );
    }

    let children = fs::read_dir(path)
        .map_err(|e| RemoveError::ReadDirFailed(path.to_path_buf(), e))?;

    let results: Vec<Result<u64, RemoveError>> = children
        .par_bridge()
        .filter_map(|entry_result| match entry_result {
            Ok(entry) => Some(fast_remove(entry.path(), config)),
            Err(e) => {
                // Log and return error for problematic directory entries
                let error = RemoveError::DirEntryFailed(path.to_path_buf(), e);
                if let Some(p) = &config.progress {
                    p.inc_error(path.to_path_buf(), error.to_string());
                } else {
                    eprintln!("  {}", error.to_string().red().dimmed());
                }
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
            if config.progress.is_none() {
                eprintln!(
                    "  {} {} error(s) in subdirectory {:?}, continuing...",
                    "Warning:".yellow(),
                    errors.len(),
                    path
                );
            }
        } else {
            // Return the first error
            return Err(errors.into_iter().next().unwrap().unwrap_err());
        }
    }

    if config.progress.is_none() {
        config.log_action(
            "Removing empty directory ",
            "Would remove empty directory ",
            path,
            colored::Color::Yellow,
        );
    }
    
    if !config.dry_run {
        match fs::remove_dir(path) {
            Ok(_) => {
                if let Some(p) = &config.progress {
                    p.inc_deleted(path.to_path_buf());
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path.to_path_buf(), err_msg);
                }
                return Err(RemoveError::RemoveDirFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path.to_path_buf());
        }
    }
    items_removed_count += 1; // Count the directory itself
    Ok(items_removed_count)
}

/// Main entry point for removing a path (file, directory, or symlink)
fn fast_remove(path_ref: impl AsRef<Path>, config: &RemoveConfig) -> Result<u64, RemoveError> {
    let path = path_ref.as_ref();

    if let Some(p) = &config.progress {
        p.inc_scanned();
    } else {
        config.log_check(path);
    }

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
            verbosity: 1, // Standard mode
            dry_run: false,
            threads: None,
            continue_on_error: true,
        };

        let config = RemoveConfig::from_cli(&cli, None);
        assert_eq!(config.verbosity, Verbosity::Standard);
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
            verbosity: Verbosity::Simple,
            dry_run: true,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: true,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
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
            verbosity: Verbosity::Simple,
            dry_run: false,
            continue_on_error: true, // Continue despite errors
            progress: None,
        };

        let result = fast_remove(&test_dir, &config);

        // Should succeed with continue_on_error
        // (Though some items might fail to delete due to permissions)
        assert!(result.is_ok());
    }
}
