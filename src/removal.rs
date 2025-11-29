use colored::*;
use rayon::prelude::*;
use std::fs;
use std::path::Path;

use crate::config::RemoveConfig;
use crate::errors::RemoveError;

pub fn remove_symlink(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
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
                    p.inc_deleted(path);
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path, err_msg);
                }
                return Err(RemoveError::RemoveFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path);
        }
    }
    Ok(1)
}

pub fn remove_file(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
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
                    p.inc_deleted(path);
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path, err_msg);
                }
                return Err(RemoveError::RemoveFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path);
        }
    }
    Ok(1)
}

pub fn remove_directory(path: &Path, config: &RemoveConfig) -> Result<u64, RemoveError> {
    if config.progress.is_none() {
        config.log_action(
            "Entering directory ",
            "Would enter directory ",
            path,
            colored::Color::Blue,
        );
    }

    let children =
        fs::read_dir(path).map_err(|e| RemoveError::ReadDirFailed(path.to_path_buf(), e))?;

    let results: Vec<Result<u64, RemoveError>> = children
        .par_bridge()
        .filter_map(|entry_result| match entry_result {
            Ok(entry) => Some(fast_remove(entry.path(), config)),
            Err(e) => {
                let error = RemoveError::DirEntryFailed(path.to_path_buf(), e);
                if let Some(p) = &config.progress {
                    p.inc_error(path, error.to_string());
                } else {
                    eprintln!("  {}", error.to_string().red().dimmed());
                }
                Some(Err(error))
            }
        })
        .collect();

    let (successes, errors): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);
    let mut items_removed_count: u64 = successes.into_iter().map(|r| r.unwrap()).sum();

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
                    p.inc_deleted(path);
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(p) = &config.progress {
                    p.inc_error(path, err_msg);
                }
                return Err(RemoveError::RemoveDirFailed(path.to_path_buf(), e));
            }
        }
    } else {
        if let Some(p) = &config.progress {
            p.inc_deleted(path);
        }
    }

    items_removed_count += 1;
    Ok(items_removed_count)
}

pub fn fast_remove(path_ref: impl AsRef<Path>, config: &RemoveConfig) -> Result<u64, RemoveError> {
    let path = path_ref.as_ref();
    if let Some(p) = &config.progress {
        p.inc_scanned();
    } else {
        config.log_check(path);
    }

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
    use tempfile::TempDir;

    #[test]
    fn test_dry_run_does_not_delete() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        std::fs::write(&test_file, "test content").unwrap();
        assert!(test_file.exists());

        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: true,
            continue_on_error: false,
            progress: None,
        };
        let result = remove_file(&test_file, &config);
        assert!(result.is_ok());
        assert!(test_file.exists(), "Dry-run should not delete files");
    }

    #[test]
    fn test_dry_run_directory() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("testdir");
        std::fs::create_dir(&test_dir).unwrap();

        let config = RemoveConfig {
            verbosity: crate::config::Verbosity::Simple,
            dry_run: true,
            continue_on_error: false,
            progress: None,
        };
        let result = fast_remove(&test_dir, &config);
        assert!(result.is_ok());
        assert!(test_dir.exists(), "Dry-run should not delete directories");
    }
}
