use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FsError, FsResult, Metadata, NormalizedPath};

/// File descriptor identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fd(u32);

impl Fd {
    /// Standard input.
    pub const STDIN: Self = Self(0);
    /// Standard output.
    pub const STDOUT: Self = Self(1);
    /// Standard error.
    pub const STDERR: Self = Self(2);

    /// Creates a file descriptor id.
    #[must_use]
    pub fn new(fd: u32) -> Self {
        Self(fd)
    }

    /// Returns the numeric fd.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Open file stored in a task fd table.
#[derive(Clone)]
pub struct OpenFile {
    file: Arc<Mutex<Box<dyn File>>>,
    path: NormalizedPath,
}

impl OpenFile {
    /// Creates an open fd entry.
    #[must_use]
    pub fn new(file: Box<dyn File>, path: NormalizedPath) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
            path,
        }
    }

    /// Reads from the open file.
    pub fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("fd file lock poisoned".to_owned()))?
            .read(buf)
    }

    /// Writes to the open file.
    pub fn write(&self, buf: &[u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("fd file lock poisoned".to_owned()))?
            .write(buf)
    }

    /// Returns file metadata.
    pub fn metadata(&self) -> FsResult<Metadata> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("fd file lock poisoned".to_owned()))?
            .metadata()
    }

    /// Returns the original path associated with the open fd.
    #[must_use]
    pub fn path(&self) -> &NormalizedPath {
        &self.path
    }
}

/// Per-task fd table.
#[derive(Default)]
pub struct FdTable {
    files: BTreeMap<Fd, OpenFile>,
    next_fd: u32,
}

impl fmt::Debug for FdTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FdTable")
            .field("fds", &self.fds())
            .field("next_fd", &self.next_fd)
            .finish()
    }
}

impl FdTable {
    /// Creates an empty fd table. Dynamic fds start at 3.
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            next_fd: 3,
        }
    }

    /// Opens a file into the next dynamic fd.
    pub fn open(&mut self, file: Box<dyn File>, path: NormalizedPath) -> Fd {
        let fd = Fd::new(self.next_fd);
        self.next_fd += 1;
        self.files.insert(fd, OpenFile::new(file, path));
        fd
    }

    /// Installs or replaces a specific fd entry.
    pub fn insert_at(&mut self, fd: Fd, file: Box<dyn File>, path: NormalizedPath) {
        self.next_fd = self.next_fd.max(fd.get().saturating_add(1));
        self.files.insert(fd, OpenFile::new(file, path));
    }

    /// Closes an fd by removing it from the table.
    ///
    /// # Errors
    ///
    /// Returns an invalid-fd error when the fd is not open.
    pub fn close(&mut self, fd: Fd) -> FsResult<()> {
        self.files.remove(&fd).map(|_| ()).ok_or(FsError::InvalidFd)
    }

    /// Reads from an open fd.
    ///
    /// # Errors
    ///
    /// Returns an invalid-fd error when the fd is not open, or a filesystem
    /// error from the underlying file.
    pub fn read(&mut self, fd: Fd, buf: &mut [u8]) -> FsResult<usize> {
        self.file(fd)?.read(buf)
    }

    /// Writes to an open fd.
    ///
    /// # Errors
    ///
    /// Returns an invalid-fd error when the fd is not open, or a filesystem
    /// error from the underlying file.
    pub fn write(&mut self, fd: Fd, buf: &[u8]) -> FsResult<usize> {
        self.file(fd)?.write(buf)
    }

    /// Returns metadata for an open fd.
    pub fn metadata(&self, fd: Fd) -> FsResult<Metadata> {
        self.file(fd)?.metadata()
    }

    /// Returns the path originally associated with an fd.
    pub fn path(&self, fd: Fd) -> FsResult<NormalizedPath> {
        Ok(self.file(fd)?.path().clone())
    }

    /// Returns a cloneable open-file handle for an fd.
    pub fn file(&self, fd: Fd) -> FsResult<OpenFile> {
        self.files.get(&fd).cloned().ok_or(FsError::InvalidFd)
    }

    /// Returns the sorted open fd numbers.
    #[must_use]
    pub fn fds(&self) -> Vec<Fd> {
        self.files.keys().copied().collect()
    }
}
