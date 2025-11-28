use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use colored::*;
use rayon::prelude::*;

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

/// Deduplicate and check for overlapping paths to prevent concurrent access issues
fn deduplicate_and_check_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
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
                return Err(format!(
                    "Path overlap detected: {:?} is inside {:?}. This could cause concurrent access issues.",
                    path_i, path_j
                ));
            }
            if path_j.starts_with(path_i) {
                return Err(format!(
                    "Path overlap detected: {:?} is inside {:?}. This could cause concurrent access issues.",
                    path_j, path_i
                ));
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

    if cli.dry_run {
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
            if cli.verbose || cli.dry_run {
                println!(
                    "{} {:?}...",
                    if cli.dry_run {
                        "Would process".blue()
                    } else {
                        "Processing".cyan()
                    },
                    path
                );
            }
            let result = fast_remove(path, cli.verbose, cli.dry_run, cli.continue_on_error);
            (path, result)
        })
        .collect();

    let mut total_errors = 0;
    let mut total_items = 0;

    for (path, result) in results {
        match result {
            Ok(count) => {
                total_items += count;
                if count > 0 || cli.verbose {
                    // Only print success if something was (or would be) done, or if verbose
                    println!(
                        "{} {:?} ({} {} {})",
                        if cli.dry_run {
                            "Would successfully remove".green()
                        } else {
                            "Successfully removed".green()
                        },
                        path,
                        count,
                        if count == 1 { "item" } else { "items" },
                        if cli.dry_run { "processed" } else { "deleted" }
                    );
                }
            }
            Err(e) => {
                total_errors += 1;
                eprintln!("{} {:?}: {}", "Failed to remove".red(), path, e.red());
            }
        }
    }

    if cli.dry_run {
        println!("{}", "Dry run finished.".yellow().bold());
    }

    if total_items > 0 || cli.verbose {
        println!(
            "\n{} {} total {} {}.",
            "Summary:".bold(),
            total_items,
            if total_items == 1 { "item" } else { "items" },
            if cli.dry_run { "would be removed" } else { "removed" }
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
}

// Returns the number of items (files/symlinks/dirs) processed/deleted.
fn fast_remove(
    path_ref: impl AsRef<Path>,
    verbose: bool,
    dry_run: bool,
    continue_on_error: bool,
) -> Result<u64, String> {
    let path = path_ref.as_ref();
    let mut items_removed_count = 0;

    if verbose {
        println!(
            "  {}{:?}",
            if dry_run { "Would check " } else { "Checking " }.dimmed(),
            path
        );
    }

    // Use symlink_metadata to correctly assess symlinks, even broken ones.
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Failed to get metadata for {:?}: {}", path, e))?;

    if metadata.file_type().is_symlink() {
        if verbose || dry_run {
            println!(
                "  {}{:?}",
                if dry_run {
                    "Would remove symlink "
                } else {
                    "Removing symlink "
                }
                .yellow(),
                path
            );
        }
        if !dry_run {
            fs::remove_file(path)
                .map_err(|e| format!("Failed to remove symlink {:?}: {}", path, e))?;
        }
        items_removed_count += 1;
        return Ok(items_removed_count);
    }

    if metadata.is_file() {
        if verbose || dry_run {
            println!(
                "  {}{:?}",
                if dry_run {
                    "Would remove file "
                } else {
                    "Removing file "
                }
                .yellow(),
                path
            );
        }
        if !dry_run {
            fs::remove_file(path)
                .map_err(|e| format!("Failed to remove file {:?}: {}", path, e))?;
        }
        items_removed_count += 1;
        return Ok(items_removed_count);
    }

    if metadata.is_dir() {
        if verbose || dry_run {
            println!(
                "  {}{:?}",
                if dry_run {
                    "Would enter directory "
                } else {
                    "Entering directory "
                }
                .blue(),
                path
            );
        }

        let children = match fs::read_dir(path) {
            Ok(children) => children,
            Err(e) => return Err(format!("Failed to read directory {:?}: {}", path, e)),
        };

        let results: Vec<Result<u64, String>> = children
            .par_bridge()
            .filter_map(|entry_result| match entry_result {
                Ok(entry) => Some(fast_remove(entry.path(), verbose, dry_run, continue_on_error)),
                Err(e) => {
                    // Log and return error for problematic directory entries
                    let error_msg = format!("Error accessing directory entry in {:?}: {}", path, e);
                    eprintln!("  {}", error_msg.red().dimmed());
                    Some(Err(error_msg))
                }
            })
            .collect();

        let mut errors = Vec::new();
        for result in results {
            match result {
                Ok(count) => items_removed_count += count,
                Err(e) => {
                    if continue_on_error {
                        errors.push(e);
                    } else {
                        return Err(e); // Propagate error immediately
                    }
                }
            }
        }

        // If there were errors and we're continuing, report them but don't fail the whole operation
        if !errors.is_empty() && continue_on_error {
            eprintln!(
                "  {} {} error(s) in subdirectory {:?}, continuing...",
                "Warning:".yellow(),
                errors.len(),
                path
            );
        }

        if verbose || dry_run {
            println!(
                "  {}{:?}",
                if dry_run {
                    "Would remove empty directory "
                } else {
                    "Removing empty directory "
                }
                .yellow(),
                path
            );
        }
        if !dry_run {
            fs::remove_dir(path)
                .map_err(|e| format!("Failed to remove directory {:?}: {}", path, e))?;
        }
        items_removed_count += 1; // Count the directory itself
        return Ok(items_removed_count);
    }

    Err(format!(
        "Path {:?} is not a file, directory, or symlink that can be removed.",
        path
    ))
}
