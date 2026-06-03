//! Filesystem contracts for the Rust-native Wanix port.
//!
//! This crate owns path rules, metadata, file traits, filesystem traits, and
//! errors. Namespace and task behavior live in higher-level crates.

use std::error::Error;
use std::fmt;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix filesystem contracts";

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
            Self::NotEmpty => f.write_str("directory not empty"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl Error for FsError {}

/// Wanix file path normalized to the `io/fs.ValidPath` shape used by Go.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedPath(String);

impl NormalizedPath {
    /// Creates a normalized path after validating relative path components.
    ///
    /// `.` is the filesystem root. Other paths must be slash-separated,
    /// relative, non-empty, and contain no `.` or `..` components.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidPath`] when the input is not a valid Wanix
    /// filesystem path.
    pub fn new(path: impl AsRef<str>) -> FsResult<Self> {
        let path = path.as_ref();
        if is_valid_path(path) {
            Ok(Self(path.to_owned()))
        } else {
            Err(FsError::InvalidPath(path.to_owned()))
        }
    }

    /// Returns the normalized path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NormalizedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn is_valid_path(path: &str) -> bool {
    if path == "." {
        return true;
    }
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        return false;
    }
    path.split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

/// Broad file kind used by Wanix metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Regular byte file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
}

/// File metadata shared by directory entries and stat-like operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    file_type: FileType,
    len: u64,
    mode: u32,
}

impl Metadata {
    /// Creates metadata from a file type, byte length, and Unix-style mode.
    #[must_use]
    pub fn new(file_type: FileType, len: u64, mode: u32) -> Self {
        Self {
            file_type,
            len,
            mode,
        }
    }

    /// Returns the broad file kind.
    #[must_use]
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the byte length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns whether the file has no byte content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the Unix-style mode bits currently associated with this file.
    #[must_use]
    pub fn mode(&self) -> u32 {
        self.mode
    }
}

/// Directory entry returned by readdir-like operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    name: String,
    metadata: Metadata,
}

impl DirEntry {
    /// Creates a directory entry.
    #[must_use]
    pub fn new(name: impl Into<String>, metadata: Metadata) -> Self {
        Self {
            name: name.into(),
            metadata,
        }
    }

    /// Returns the entry basename.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the entry metadata.
    #[must_use]
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

/// Open options used by filesystem implementations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// Open for reading.
    pub read: bool,
    /// Open for writing.
    pub write: bool,
    /// Create the file if it is missing.
    pub create: bool,
    /// Truncate the file after opening.
    pub truncate: bool,
}

/// Open file behavior.
pub trait File: Send {
    /// Reads bytes into `buf`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot be read.
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize>;

    /// Writes bytes from `buf`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot be written.
    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    /// Returns file metadata.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when metadata cannot be produced.
    fn metadata(&self) -> FsResult<Metadata>;
}

/// Filesystem behavior used by namespaces, tasks, and WASI adapters.
pub trait FileSystem: Send + Sync {
    /// Opens a file at `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot be opened.
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>>;

    /// Returns metadata for `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when metadata cannot be produced.
    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata>;

    /// Returns directory entries for `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot be read as a directory.
    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>>;
}

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, DirEntry, FileType, FsError, Metadata, NormalizedPath};

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn normalized_path_matches_valid_path_shape() {
        for path in [".", "#task", "dir/file", "a-b/c_d"] {
            assert_eq!(NormalizedPath::new(path).unwrap().as_str(), path);
        }

        for path in [
            "",
            "/",
            "/abs",
            "dir/",
            "dir//file",
            "dir/.",
            "../x",
            "x/../y",
        ] {
            assert!(matches!(
                NormalizedPath::new(path),
                Err(FsError::InvalidPath(_))
            ));
        }
    }

    #[test]
    fn metadata_and_entries_expose_contract_fields() {
        let metadata = Metadata::new(FileType::File, 4, 0o644);
        let entry = DirEntry::new("file.txt", metadata.clone());

        assert_eq!(entry.name(), "file.txt");
        assert_eq!(entry.metadata(), &metadata);
        assert_eq!(metadata.file_type(), FileType::File);
        assert_eq!(metadata.len(), 4);
        assert_eq!(metadata.mode(), 0o644);
        assert!(!metadata.is_empty());
    }
}
