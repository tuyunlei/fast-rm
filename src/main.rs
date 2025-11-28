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

    /// Force removal without prompting (currently a placeholder, not fully implemented)
    #[clap(short, long)]
    force: bool,

    /// Provide verbose output
    #[clap(short, long)]
    verbose: bool,

    /// Do not actually remove anything, just show what would be done
    #[clap(short = 'n', long = "dry-run")]
    dry_run: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.paths.is_empty() {
        println!(
            "{}",
            "No paths provided. Use --help for usage information.".yellow()
        );
        return;
    }

    if cli.dry_run {
        println!(
            "{}",
            "Dry run mode activated. No files will be deleted."
                .yellow()
                .bold()
        );
    }

    cli.paths.par_iter().for_each(|path| {
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
        match fast_remove(path, cli.verbose, cli.dry_run) {
            Ok(count) => {
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
                eprintln!("{} {:?}: {}", "Failed to remove".red(), path, e.red());
            }
        }
    });
    if cli.dry_run {
        println!("{}", "Dry run finished.".yellow().bold());
    }
}

// Returns the number of items (files/symlinks/dirs) processed/deleted.
fn fast_remove(path_ref: impl AsRef<Path>, verbose: bool, dry_run: bool) -> Result<u64, String> {
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
    let metadata = match fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(_) if !path.exists() => {
            // Path doesn't exist and isn't a symlink that symlink_metadata would find (e.g. truly non-existent)
            return Err(format!("Path does not exist"));
        }
        Err(e) => {
            // Other errors for symlink_metadata, or path exists but isn't a symlink
            // If it's not a symlink, path.exists() should be true if it's a file/dir
            if !path.exists() {
                return Err(format!(
                    "Path does not exist or is a broken link, and failed to get metadata: {}",
                    e
                ));
            }
            // Fallback to regular metadata if symlink_metadata failed but path exists (should be rare)
            match path.metadata() {
                Ok(md) => md,
                Err(e_reg) => {
                    return Err(format!(
                        "Failed to get metadata for {:?}: {} (symlink_metadata also failed: {})",
                        path, e_reg, e
                    ))
                }
            }
        }
    };

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
                Ok(entry) => Some(fast_remove(entry.path(), verbose, dry_run)),
                Err(e) => {
                    // Log and skip problematic entries, or return Err to halt for this directory
                    eprintln!(
                        "  {}: {:?} (entry path unknown) - {}",
                        "Error accessing directory entry".red().dimmed(),
                        path,
                        e
                    );
                    Some(Err(format!(
                        "Error accessing an entry in {:?}: {}",
                        path, e
                    ))) // Make it an error to propagate
                }
            })
            .collect();

        for result in results {
            match result {
                Ok(count) => items_removed_count += count,
                Err(e) => {
                    return Err(format!(
                        "Error processing subdirectory/file within {:?}: {}",
                        path, e
                    ));
                }
            }
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
