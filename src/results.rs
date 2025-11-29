use colored::*;
use std::path::PathBuf;

use crate::config::RemoveConfig;
use crate::errors::RemoveError;

pub fn process_results(
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
                    println!(
                        "{} {:?} ({} {} {})",
                        if config.dry_run { "Would successfully remove".green() } else { "Successfully removed".green() },
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

pub fn print_summary_and_exit(total_items: u64, total_errors: u64, config: &RemoveConfig) -> ! {
    if config.dry_run {
        println!("{}", "Dry run finished.".yellow().bold());
    }

    if total_items > 0 || config.verbosity.is_verbose() {
        println!(
            "\n{} {} total {} {}.",
            "Summary:".bold(),
            total_items,
            if total_items == 1 { "item" } else { "items" },
            if config.dry_run { "would be removed" } else { "removed" }
        );
    }

    if total_errors > 0 {
        eprintln!("{} {} error(s) encountered.", "Errors:".bold().red(), total_errors);
        std::process::exit(1);
    }

    std::process::exit(0);
}

