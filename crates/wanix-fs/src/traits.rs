use crate::{DirEntry, FsError, FsResult, Metadata, NormalizedPath};

/// How path metadata should treat a final symbolic link component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataLookup {
    /// Follow a final symbolic link and report metadata for its target.
    FollowSymlink,
    /// Report metadata for the final symbolic link itself.
    NoFollow,
}

impl MetadataLookup {
    /// Returns whether final symbolic links should be followed.
    #[must_use]
    pub const fn follow_symlinks(self) -> bool {
        matches!(self, Self::FollowSymlink)
    }
}

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

    /// Sets the file length in bytes.
    ///
    /// Implementations should preserve the current file offset when possible,
    /// matching `ftruncate`-style behavior.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the handle cannot change file size.
    fn set_len(&mut self, _len: u64) -> FsResult<()> {
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

    /// Returns metadata for `path` using the requested symbolic-link lookup mode.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when metadata cannot be produced.
    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        _lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        self.metadata(path)
    }

    /// Returns directory entries for `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot be read as a directory.
    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>>;

    /// Creates one directory at `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the directory cannot be created.
    fn create_dir(&self, _path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Removes a non-directory file at `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot be removed.
    fn remove_file(&self, _path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Removes one empty directory at `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the directory cannot be removed.
    fn remove_dir(&self, _path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Renames one file or directory path to another path in this filesystem.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot be renamed.
    fn rename(&self, _old_path: &NormalizedPath, _new_path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Sets explicit access and modification times for `path`.
    ///
    /// Timestamps are nanoseconds since the Unix epoch. Metadata-change time is
    /// filesystem-specific and may remain deterministic for virtual filesystems.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot have its timestamps updated.
    fn set_times(
        &self,
        _path: &NormalizedPath,
        _accessed_time_ns: u64,
        _modified_time_ns: u64,
    ) -> FsResult<()> {
        Err(FsError::NotSupported)
    }
}
