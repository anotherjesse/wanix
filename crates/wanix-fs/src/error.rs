use std::error::Error;
use std::fmt;

/// Result type for Wanix filesystem operations.
pub type FsResult<T> = Result<T, FsError>;

/// Filesystem errors shared across Rust Wanix crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// The path is not a valid normalized Wanix filesystem path.
    InvalidPath(String),
    /// The requested file or directory does not exist.
    NotFound,
    /// The operation is not supported by this file or filesystem.
    NotSupported,
    /// The caller lacks permission for this operation.
    PermissionDenied,
    /// The destination already exists.
    AlreadyExists,
    /// The path was expected to name a directory.
    NotDirectory,
    /// The path was expected to name a non-directory file.
    IsDirectory,
    /// The file descriptor is not open or is invalid.
    InvalidFd,
    /// A file offset is invalid for the requested operation.
    InvalidOffset,
    /// A file timestamp is invalid for the requested operation.
    InvalidTime,
    /// A directory removal failed because the directory has entries.
    NotEmpty,
    /// A descriptive fallback for errors that do not yet have a stable variant.
    Other(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::NotFound => f.write_str("file does not exist"),
            Self::NotSupported => f.write_str("operation not supported"),
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::AlreadyExists => f.write_str("file already exists"),
            Self::NotDirectory => f.write_str("not a directory"),
            Self::IsDirectory => f.write_str("is a directory"),
            Self::InvalidFd => f.write_str("invalid file descriptor"),
            Self::InvalidOffset => f.write_str("invalid file offset"),
            Self::InvalidTime => f.write_str("invalid file timestamp"),
            Self::NotEmpty => f.write_str("directory not empty"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl Error for FsError {}
