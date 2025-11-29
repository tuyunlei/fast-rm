use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::config::RemoveConfig;
use crate::errors::RemoveError;
use crate::queue::{AdaptiveQueue, FileJob};

/// Recursively scan a path and enqueue all files/directories for deletion
///
/// This function traverses the file system tree in parallel, enqueuing work items
/// for the deleter threads to process. Directories are enqueued AFTER all their
/// children to ensure correct deletion order.
pub fn scan_path(
    path: &Path,
    queue: &AdaptiveQueue,
    config: &RemoveConfig,
) -> Result<(), RemoveError> {
    // Increment scanned counter
    if let Some(p) = &config.progress {
        p.inc_scanned();
    }

    // Get metadata without following symlinks
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| RemoveError::MetadataFailed(path.to_path_buf(), e))?;

    if metadata.file_type().is_symlink() {
        // Enqueue symlink for deletion
        queue
            .send(FileJob::Symlink(Arc::from(path)))
            .map_err(|_| RemoveError::QueueFull)?;
    } else if metadata.is_file() {
        // Enqueue file for deletion
        queue
            .send(FileJob::File(Arc::from(path)))
            .map_err(|_| RemoveError::QueueFull)?;
    } else if metadata.is_dir() {
        // Recursively scan directory, then enqueue the directory itself
        scan_directory(path, queue, config)?;

        // Enqueue directory AFTER all children have been scanned
        // This ensures children are deleted before the parent
        queue
            .send(FileJob::EmptyDir(Arc::from(path)))
            .map_err(|_| RemoveError::QueueFull)?;
    } else {
        return Err(RemoveError::UnsupportedType(path.to_path_buf()));
    }

    Ok(())
}

/// Scan all entries in a directory in parallel
fn scan_directory(
    path: &Path,
    queue: &AdaptiveQueue,
    config: &RemoveConfig,
) -> Result<(), RemoveError> {
    let entries =
        fs::read_dir(path).map_err(|e| RemoveError::ReadDirFailed(path.to_path_buf(), e))?;

    // Parallel scan of directory children
    let results: Vec<Result<(), RemoveError>> = entries
        .par_bridge()
        .filter_map(|entry_result| match entry_result {
            Ok(entry) => Some(scan_path(&entry.path(), queue, config)),
            Err(e) => {
                let error = RemoveError::DirEntryFailed(path.to_path_buf(), e);
                if let Some(p) = &config.progress {
                    p.inc_error(path, error.to_string());
                } else {
                    eprintln!("  {}", error.to_string());
                }
                Some(Err(error))
            }
        })
        .collect();

    // Check for errors
    let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    if !errors.is_empty() && !config.continue_on_error {
        return Err(errors.into_iter().next().unwrap());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_scan_single_file() {
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

        scan_path(&test_file, &queue, &config).unwrap();

        // Should have one file job
        assert_eq!(queue.depth(), 1);
        match queue.recv().unwrap() {
            FileJob::File(_) => {}
            _ => panic!("Expected File job"),
        }
    }

    #[test]
    fn test_scan_directory_with_files() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("testdir");
        fs::create_dir(&test_dir).unwrap();

        // Create 3 files in the directory
        for i in 1..=3 {
            File::create(test_dir.join(format!("file{}.txt", i))).unwrap();
        }

        let queue = AdaptiveQueue::new(20);
        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
        };

        scan_path(&test_dir, &queue, &config).unwrap();

        // Should have 3 files + 1 directory = 4 jobs
        assert_eq!(queue.depth(), 4);

        // Collect all jobs
        let mut files = 0;
        let mut dirs = 0;
        while let Ok(job) = queue.try_recv() {
            match job {
                FileJob::File(_) => files += 1,
                FileJob::EmptyDir(_) => dirs += 1,
                _ => {}
            }
        }

        assert_eq!(files, 3);
        assert_eq!(dirs, 1);
    }

    #[test]
    fn test_nested_directories() {
        let temp_dir = TempDir::new().unwrap();
        let dir1 = temp_dir.path().join("dir1");
        let dir2 = dir1.join("dir2");
        fs::create_dir_all(&dir2).unwrap();

        File::create(dir2.join("file.txt")).unwrap();

        let queue = AdaptiveQueue::new(20);
        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: false,
            continue_on_error: false,
            progress: None,
        };

        scan_path(&dir1, &queue, &config).unwrap();

        // Should have: 1 file + 2 directories = 3 jobs
        assert_eq!(queue.depth(), 3);
    }
}
