use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata, NormalizedPath};
use wanix_vfs::Namespace;

use crate::{WasiFd, WasiFileAccess};

/// Configured preopen directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preopen {
    guest_path: NormalizedPath,
}

impl Preopen {
    /// Creates a preopen rooted at a Wanix namespace path.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `guest_path` is invalid.
    pub fn new(guest_path: impl AsRef<str>) -> FsResult<Self> {
        Ok(Self {
            guest_path: NormalizedPath::new(guest_path)?,
        })
    }

    /// Returns the guest path exposed as a preopen.
    #[must_use]
    pub fn guest_path(&self) -> &NormalizedPath {
        &self.guest_path
    }
}

/// Shared file handle attached to a WASI fd.
#[derive(Clone)]
pub struct WasiFile {
    file: Arc<Mutex<Box<dyn File>>>,
    label: String,
    access: WasiFileAccess,
}

impl fmt::Debug for WasiFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasiFile")
            .field("label", &self.label)
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

impl WasiFile {
    /// Creates a shared WASI fd file attachment.
    pub fn new(file: Box<dyn File>, label: impl Into<String>, access: WasiFileAccess) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
            label: label.into(),
            access,
        }
    }

    /// Creates a read-only stdin attachment.
    #[must_use]
    pub fn stdin(file: Box<dyn File>, label: impl Into<String>) -> Self {
        Self::new(file, label, WasiFileAccess::read_only())
    }

    /// Creates a write-only stdout/stderr attachment.
    #[must_use]
    pub fn output(file: Box<dyn File>, label: impl Into<String>) -> Self {
        Self::new(file, label, WasiFileAccess::write_only())
    }

    /// Returns the debug label associated with this attached file.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn can_read(&self) -> bool {
        self.access.can_read()
    }

    pub(crate) fn can_write(&self) -> bool {
        self.access.can_write()
    }

    pub(crate) fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .read(buf)
    }

    pub(crate) fn write(&self, buf: &[u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .write(buf)
    }

    pub(crate) fn seek(&self, from: FileSeekFrom) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .seek(from)
    }

    pub(crate) fn tell(&self) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .tell()
    }

    pub(crate) fn is_seekable(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))
            .map(|file| file.is_seekable())
    }

    pub(crate) fn metadata(&self) -> FsResult<Metadata> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .metadata()
    }
}

/// WASI host configuration backed by a Wanix namespace.
#[derive(Clone)]
pub struct WasiConfig {
    namespace: Namespace,
    stdio: BTreeMap<WasiFd, WasiFile>,
    preopens: Vec<Preopen>,
}

impl fmt::Debug for WasiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasiConfig")
            .field("namespace", &self.namespace)
            .field("stdio_fds", &self.stdio.keys().collect::<Vec<_>>())
            .field("preopens", &self.preopens)
            .finish()
    }
}

impl WasiConfig {
    /// Creates a config for a namespace.
    #[must_use]
    pub fn new(namespace: Namespace) -> Self {
        Self {
            namespace,
            stdio: BTreeMap::new(),
            preopens: vec![Preopen {
                guest_path: NormalizedPath::new(".").expect("root path is valid"),
            }],
        }
    }

    /// Returns the Wanix namespace backing the adapter.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Returns configured preopens.
    #[must_use]
    pub fn preopens(&self) -> &[Preopen] {
        &self.preopens
    }

    /// Returns configured standard fd numbers.
    #[must_use]
    pub fn stdio_fds(&self) -> Vec<WasiFd> {
        self.stdio.keys().copied().collect()
    }

    pub(crate) fn stdio(&self) -> &BTreeMap<WasiFd, WasiFile> {
        &self.stdio
    }

    /// Attaches a stdin file to fd 0.
    #[must_use]
    pub fn with_stdin(self, file: Box<dyn File>, label: impl Into<String>) -> Self {
        self.with_stdio(WasiFd::STDIN, WasiFile::stdin(file, label))
    }

    /// Attaches a stdout file to fd 1.
    #[must_use]
    pub fn with_stdout(self, file: Box<dyn File>, label: impl Into<String>) -> Self {
        self.with_stdio(WasiFd::STDOUT, WasiFile::output(file, label))
    }

    /// Attaches a stderr file to fd 2.
    #[must_use]
    pub fn with_stderr(self, file: Box<dyn File>, label: impl Into<String>) -> Self {
        self.with_stdio(WasiFd::STDERR, WasiFile::output(file, label))
    }

    fn with_stdio(mut self, fd: WasiFd, file: WasiFile) -> Self {
        self.stdio.insert(fd, file);
        self
    }

    /// Adds a preopen and returns the updated config.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `guest_path` is invalid.
    pub fn with_preopen(mut self, guest_path: impl AsRef<str>) -> FsResult<Self> {
        self.preopens.push(Preopen::new(guest_path)?);
        Ok(self)
    }
}

impl Default for WasiConfig {
    fn default() -> Self {
        Self::new(Namespace::new())
    }
}
