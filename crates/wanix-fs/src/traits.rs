use crate::{DirEntry, FsError, FsResult, Metadata, NormalizedPath};

/// File seek origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSeekFrom {
    /// Seek to an absolute byte offset from the beginning of the file.
    Start(u64),
    /// Seek relative to the current file offset.
    Current(i64),
    /// Seek relative to the current end of file.
    End(i64),
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

impl OpenOptions {
    /// Read-only open options.
    #[must_use]
    pub fn read() -> Self {
        Self {
            read: true,
            ..Self::default()
        }
    }

    /// Read-write open options.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            ..Self::default()
        }
    }
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

    /// Seeks to a new file offset and returns the resulting absolute offset.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot seek or the requested
    /// offset is invalid.
    fn seek(&mut self, _from: FileSeekFrom) -> FsResult<u64> {
        Err(FsError::NotSupported)
    }

    /// Returns the current file offset.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot report its offset.
    fn tell(&self) -> FsResult<u64> {
        Err(FsError::NotSupported)
    }

    /// Returns whether this handle supports seek/tell operations.
    #[must_use]
    fn is_seekable(&self) -> bool {
        false
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
