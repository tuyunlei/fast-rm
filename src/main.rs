use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use clap::Parser;
use colored::*;
use rayon::prelude::*;

mod cli;
mod config;
mod errors;
mod path;
mod progress;
mod queue;
mod removal;
mod results;
mod scanner;

use crate::cli::Cli;
use crate::config::{RemoveConfig, Verbosity};
use crate::path::deduplicate_and_check_paths;
use crate::progress::{ProgressDisplay, RemoveProgress};
use crate::removal::fast_remove;
use crate::results::{print_summary_and_exit, process_results};

fn main() {
    let cli = Cli::parse();

    if let Some(num_threads) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()
            .expect("Failed to initialize thread pool");
    }

    let paths_to_process = match deduplicate_and_check_paths(&cli.paths) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    };

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
        println!();
    }

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

    is_done.store(true, Ordering::Relaxed);
    tui_thread.join().expect("TUI thread panicked");
    display.finish(&progress, cli.dry_run);

    let (total_items, total_errors) = process_results(results, &config);
    print_summary_and_exit(total_items, total_errors, &config);
}
