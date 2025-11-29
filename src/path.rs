use std::collections::HashSet;
use std::path::PathBuf;

use colored::*;

use crate::errors::RemoveError;

pub fn deduplicate_and_check_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, RemoveError> {
    let mut canonical_paths = Vec::new();
    let mut seen = HashSet::new();

    for path in paths {
        match path.canonicalize() {
            Ok(canonical) => {
                if !seen.contains(&canonical) {
                    seen.insert(canonical.clone());
                    canonical_paths.push(canonical);
                }
            }
            Err(e) => {
                eprintln!("{} Failed to canonicalize {:?}: {}. Using original path.", "Warning:".yellow(), path, e);
                if !seen.contains(path) {
                    seen.insert(path.clone());
                    canonical_paths.push(path.clone());
                }
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs::File;

    #[test]
    fn test_path_deduplication() {
        let temp_dir = TempDir::new().unwrap();
        let path1 = temp_dir.path().join("file.txt");
        let path2 = temp_dir.path().join("file.txt");
        File::create(&path1).unwrap();
        let paths = vec![path1.clone(), path2.clone()];
        let result = deduplicate_and_check_paths(&paths).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_path_overlap_detection() {
        let temp_dir = TempDir::new().unwrap();
        let parent = temp_dir.path().join("parent");
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let paths = vec![parent.clone(), child.clone()];
        let result = deduplicate_and_check_paths(&paths);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, RemoveError::PathOverlap(_)));
    }
}

