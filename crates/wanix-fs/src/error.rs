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
    /// A live resource (e.g. a mesh-mounted peer) is temporarily unreachable
    /// at the transport level: the resource is unusable right now, not
    /// missing. Maps to `EIO`-class errors, never `ENOENT` (ADR 0008). The
    /// payload is diagnostic detail, never something callers should match on.
    Unreachable(String),
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
            Self::Unreachable(detail) => write!(f, "resource unreachable: {detail}"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl Error for FsError {}

#[cfg(test)]
mod tests {
    use super::FsError;

    #[test]
    fn fs_error_display_messages_are_stable() {
        let cases = [
            (
                FsError::InvalidPath("bad/../path".to_owned()),
                "invalid path: bad/../path",
            ),
            (FsError::NotFound, "file does not exist"),
            (FsError::NotSupported, "operation not supported"),
            (FsError::PermissionDenied, "permission denied"),
            (FsError::AlreadyExists, "file already exists"),
            (FsError::NotDirectory, "not a directory"),
            (FsError::IsDirectory, "is a directory"),
            (FsError::InvalidFd, "invalid file descriptor"),
            (FsError::InvalidOffset, "invalid file offset"),
            (FsError::InvalidTime, "invalid file timestamp"),
            (FsError::NotEmpty, "directory not empty"),
            (
                FsError::Unreachable("peer is offline".to_owned()),
                "resource unreachable: peer is offline",
            ),
            (FsError::Other("host detail".to_owned()), "host detail"),
        ];

        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }
}
