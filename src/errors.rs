use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum RemoveError {
    MetadataFailed(PathBuf, io::Error),
    RemoveFailed(PathBuf, io::Error),
    ReadDirFailed(PathBuf, io::Error),
    RemoveDirFailed(PathBuf, io::Error),
    DirEntryFailed(PathBuf, io::Error),
    UnsupportedType(PathBuf),
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
                write!(f, "Path {:?} is not a file, directory, or symlink that can be removed", path)
            }
            RemoveError::PathOverlap(msg) => write!(f, "{}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_error_display() {
        let path = PathBuf::from("/tmp/test");
        let error = RemoveError::UnsupportedType(path.clone());
        let display = format!("{}", error);
        assert!(display.contains("/tmp/test"));
        assert!(display.contains("not a file, directory, or symlink"));
    }
}

