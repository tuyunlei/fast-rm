use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use clap::Parser;
use colored::*;
use rayon::prelude::*;

mod cli;
mod config;
mod deleter;
mod errors;
mod path;
mod progress;
mod queue;
mod removal;
mod results;
mod scanner;

use crate::cli::Cli;
use crate::config::{RemoveConfig, Verbosity};
use crate::deleter::delete_worker;
use crate::path::deduplicate_and_check_paths;
use crate::progress::{ProgressDisplay, RemoveProgress};
use crate::queue::AdaptiveQueue;
use crate::results::print_summary_and_exit;
use crate::scanner::scan_path;

fn main() {
    let cli = Cli::parse();

    // Get thread pool sizes from CLI
    let scan_threads = cli.get_scan_threads();
    let delete_threads = cli.get_delete_threads();

    // Deduplicate and validate paths
    let paths_to_process = match deduplicate_and_check_paths(&cli.paths) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    // Initialize progress tracking and configuration
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

    // Create adaptive queue for coordinating scan/delete
    let queue_capacity = (scan_threads * 1000).max(10000);
    let queue = Arc::new(AdaptiveQueue::new(queue_capacity));

    // Signal for coordinating scanner/deleter shutdown
    let scanners_done = Arc::new(AtomicBool::new(false));

    // Spawn scanner thread pool
    let queue_scan = queue.clone();
    let config_scan = config.clone();
    let paths_scan = paths_to_process.clone();
    let scanners_done_clone = scanners_done.clone();

    let scanner_thread = thread::spawn(move || {
        // Create a custom rayon thread pool for scanning
        let scan_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(scan_threads)
            .thread_name(|i| format!("scanner-{}", i))
            .build()
            .expect("Failed to create scanner thread pool");

        // Scan all paths in parallel
        scan_pool.install(|| {
            paths_scan.par_iter().for_each(|path| {
                if let Err(e) = scan_path(path, &queue_scan, &config_scan) {
                    eprintln!("{} {}", "Scan error:".red().bold(), e);
                }
            });
        });

        // Signal that scanning is complete
        scanners_done_clone.store(true, Ordering::Release);
    });

    // Spawn deleter worker threads
    let mut deleter_threads = Vec::new();
    for i in 0..delete_threads {
        let queue_delete = queue.clone();
        let config_delete = config.clone();
        let scanners_done_delete = scanners_done.clone();

        let deleter = thread::spawn(move || {
            if config_delete.verbosity.is_verbose() && config_delete.progress.is_none() {
                println!("Deleter worker {} started", i);
            }
            delete_worker(&queue_delete, &config_delete, &scanners_done_delete);
        });

        deleter_threads.push(deleter);
    }

    // Spawn TUI thread with queue depth tracking
    let display_clone = display.clone();
    let progress_clone = progress.clone();
    let queue_clone = queue.clone();
    let dry_run = cli.dry_run;
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_clone = is_done.clone();

    let tui_thread = thread::spawn(move || {
        while !is_done_clone.load(Ordering::Relaxed) {
            let depth = queue_clone.depth();
            display_clone.update(&progress_clone, dry_run, Some(depth));
            thread::sleep(Duration::from_millis(50));
        }
        let depth = queue_clone.depth();
        display_clone.update(&progress_clone, dry_run, Some(depth));
    });

    // Wait for scanner to finish
    scanner_thread.join().expect("Scanner thread panicked");

    // Wait for all deleter threads to finish
    for deleter in deleter_threads {
        deleter.join().expect("Deleter thread panicked");
    }

    // Signal TUI to finish
    is_done.store(true, Ordering::Relaxed);
    tui_thread.join().expect("TUI thread panicked");

    // Display final summary
    let final_depth = queue.depth();
    display.finish(&progress, cli.dry_run, Some(final_depth));

    let total_errors = progress.errors.load(Ordering::Relaxed) as u64;
    let total_items = if cli.dry_run {
        progress.scanned.load(Ordering::Relaxed) as u64
    } else {
        progress.deleted.load(Ordering::Relaxed) as u64
    };

    print_summary_and_exit(total_items, total_errors, &config);
}
