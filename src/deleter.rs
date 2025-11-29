use colored::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::RemoveConfig;
use crate::errors::RemoveError;
use crate::queue::{AdaptiveQueue, FileJob};

/// Worker function that consumes FileJob items from the queue and deletes them
pub fn delete_worker(
    queue: &AdaptiveQueue,
    config: &RemoveConfig,
    scanners_done: &AtomicBool,
) {
    loop {
        match queue.recv_timeout(Duration::from_millis(100)) {
            Ok(job) => {
                let result = match job {
                    FileJob::File(path) => delete_file(&path, config),
                    FileJob::Symlink(path) => delete_symlink(&path, config),
                    FileJob::EmptyDir(path) => delete_empty_dir(&path, config),
                };

                // Handle errors
                if let Err(e) = result {
                    if !config.continue_on_error {
                        // In the two-pool architecture, we can't easily stop scanners
                        // For now, just log the error and continue
                        if config.progress.is_none() {
                            eprintln!("{}", e.to_string().red());
                        }
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // Check if scanners are done AND queue is empty
                if scanners_done.load(Ordering::Relaxed) && queue.is_empty() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                // Channel closed, exit
                break;
            }
        }
    }
}

/// Delete a single file
fn delete_file(path: &Path, config: &RemoveConfig) -> Result<(), RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Removing file ",
            "Would remove file ",
            path,
            colored::Color::Yellow,
        );
    }

    if !config.dry_run {
        fs::remove_file(path).map_err(|e| {
            let err_msg = e.to_string();
            if let Some(p) = &config.progress {
                p.inc_error(path, err_msg);
            }
            RemoveError::RemoveFailed(path.to_path_buf(), e)
        })?;
    }

    if let Some(p) = &config.progress {
        p.inc_deleted(path);
    }

    Ok(())
}

/// Delete a symlink
fn delete_symlink(path: &Path, config: &RemoveConfig) -> Result<(), RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Removing symlink ",
            "Would remove symlink ",
            path,
            colored::Color::Yellow,
        );
    }

    if !config.dry_run {
        fs::remove_file(path).map_err(|e| {
            let err_msg = e.to_string();
            if let Some(p) = &config.progress {
                p.inc_error(path, err_msg);
            }
            RemoveError::RemoveFailed(path.to_path_buf(), e)
        })?;
    }

    if let Some(p) = &config.progress {
        p.inc_deleted(path);
    }

    Ok(())
}

/// Delete an empty directory (children already deleted by queue ordering)
fn delete_empty_dir(path: &Path, config: &RemoveConfig) -> Result<(), RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Removing empty directory ",
            "Would remove empty directory ",
            path,
            colored::Color::Yellow,
        );
    }

    if !config.dry_run {
        fs::remove_dir(path).map_err(|e| {
            let err_msg = e.to_string();
            if let Some(p) = &config.progress {
                p.inc_error(path, err_msg);
            }
            RemoveError::RemoveDirFailed(path.to_path_buf(), e)
        })?;
    }

    if let Some(p) = &config.progress {
        p.inc_deleted(path);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn test_delete_file() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        File::create(&test_file).unwrap();
        assert!(test_file.exists());

        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
        };

        delete_file(&test_file, &config).unwrap();
        assert!(!test_file.exists());
    }

    #[test]
    fn test_delete_file_dry_run() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        File::create(&test_file).unwrap();

        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: true,
            continue_on_error: false,
            progress: None,
        };

        delete_file(&test_file, &config).unwrap();
        assert!(test_file.exists(), "Dry run should not delete files");
    }

    #[test]
    fn test_delete_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("testdir");
        std::fs::create_dir(&test_dir).unwrap();
        assert!(test_dir.exists());

        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
        };

        delete_empty_dir(&test_dir, &config).unwrap();
        assert!(!test_dir.exists());
    }

    #[test]
    fn test_delete_worker_basic() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        File::create(&test_file).unwrap();

        let queue = AdaptiveQueue::new(10);
        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
        };
        let scanners_done = AtomicBool::new(false);

        // Enqueue file
        let path: Arc<Path> = Arc::from(test_file.as_path());
        queue.send(FileJob::File(path)).unwrap();

        // Mark scanners as done
        scanners_done.store(true, Ordering::Relaxed);

        // Run worker
        delete_worker(&queue, &config, &scanners_done);

        // File should be deleted
        assert!(!test_file.exists());
    }
}
