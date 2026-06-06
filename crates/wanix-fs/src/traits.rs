use crate::{DirEntry, File, FsError, FsResult, Metadata, NormalizedPath, OpenOptions};

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

    /// Reads the uninterpreted target bytes of a symbolic link at `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the link cannot be read.
    fn read_link(&self, _path: &NormalizedPath) -> FsResult<Vec<u8>> {
        Err(FsError::NotSupported)
    }

    /// Creates a symbolic link at `path` with the given target bytes.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the link cannot be created.
    fn symlink(&self, _target: &[u8], _path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Creates a hard link from `new_path` to the existing file at `old_path`.
    ///
    /// Implementations should reject directories and cross-filesystem links.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the link cannot be created.
    fn hard_link(&self, _old_path: &NormalizedPath, _new_path: &NormalizedPath) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

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

    /// Sets Unix-style permission bits for `path`.
    ///
    /// Implementations should preserve file-type bits and apply the lower
    /// permission/special-mode bits that make sense for the backing filesystem.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path cannot have its permissions updated.
    fn set_permissions(&self, path: &NormalizedPath, _permissions: u32) -> FsResult<()> {
        self.metadata(path)?;
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
