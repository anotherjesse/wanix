use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata, NormalizedPath};
use wanix_vfs::Namespace;

use crate::{Errno, WasiFd, WasiFileAccess};

/// Default deterministic timestamp used for WASI clock-derived operations.
///
/// This matches the engine crate's default `clock_time_get` value so standalone
/// Wanix WASI contexts and QuickJS-hosted contexts start from the same policy.
pub const DEFAULT_CLOCK_TIME_NS: u64 = 1_700_000_000_000_000_000;

/// Configured preopen directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preopen {
    source_path: NormalizedPath,
    guest_path: NormalizedPath,
}

impl Preopen {
    /// Creates a preopen rooted at a Wanix namespace path and reported at the same guest path.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `path` is invalid.
    pub fn new(path: impl AsRef<str>) -> FsResult<Self> {
        let path = NormalizedPath::new(path)?;
        Ok(Self {
            source_path: path.clone(),
            guest_path: path,
        })
    }

    /// Creates a preopen resolved at `source_path` and reported as `guest_path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when either path is invalid.
    pub fn mapped(source_path: impl AsRef<str>, guest_path: impl AsRef<str>) -> FsResult<Self> {
        Ok(Self {
            source_path: NormalizedPath::new(source_path)?,
            guest_path: NormalizedPath::new(guest_path)?,
        })
    }

    /// Returns the Wanix namespace path backing this preopen.
    #[must_use]
    pub fn source_path(&self) -> &NormalizedPath {
        &self.source_path
    }

    /// Returns the guest path reported as a preopen.
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
    access: Arc<Mutex<WasiFileAccess>>,
}

impl fmt::Debug for WasiFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let access = self.access.lock().map(|access| *access).ok();
        f.debug_struct("WasiFile")
            .field("label", &self.label)
            .field("access", &access)
            .finish_non_exhaustive()
    }
}

impl WasiFile {
    /// Creates a shared WASI fd file attachment.
    pub fn new(file: Box<dyn File>, label: impl Into<String>, access: WasiFileAccess) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
            label: label.into(),
            access: Arc::new(Mutex::new(access)),
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
        self.access().is_ok_and(WasiFileAccess::can_read)
    }

    pub(crate) fn can_write(&self) -> bool {
        self.access().is_ok_and(WasiFileAccess::can_write)
    }

    pub(crate) fn set_append(&self, append: bool) -> FsResult<()> {
        let mut access = self.access.lock().map_err(access_lock_poisoned)?;
        *access = access.with_append(append);
        Ok(())
    }

    pub(crate) fn read_bytes(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .read(buf)
    }

    pub(crate) fn read_ready_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .read_ready()
    }

    pub(crate) fn write_bytes(&self, buf: &[u8]) -> FsResult<usize> {
        let append = self.access()?.append();
        let mut file = self
            .file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?;
        if append {
            file.seek(FileSeekFrom::End(0))?;
        }
        file.write(buf)
    }

    pub(crate) fn write_ready_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .write_ready()
    }

    pub(crate) fn seek_file(&self, from: FileSeekFrom) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .seek(from)
    }

    pub(crate) fn tell_file(&self) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .tell()
    }

    pub(crate) fn is_seekable_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))
            .map(|file| file.is_seekable())
    }

    pub(crate) fn metadata_file(&self) -> FsResult<Metadata> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .metadata()
    }

    pub(crate) fn set_len_file(&self, len: u64) -> FsResult<()> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .set_len(len)
    }

    fn access(&self) -> FsResult<WasiFileAccess> {
        self.access
            .lock()
            .map(|access| *access)
            .map_err(access_lock_poisoned)
    }
}

fn access_lock_poisoned<T>(_: T) -> FsError {
    FsError::Other("WASI fd access lock poisoned".to_owned())
}

impl File for WasiFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.can_read() {
            return Err(FsError::PermissionDenied);
        }
        self.read_bytes(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.write_bytes(buf)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.seek_file(from)
    }

    fn tell(&self) -> FsResult<u64> {
        self.tell_file()
    }

    fn is_seekable(&self) -> bool {
        self.is_seekable_file().unwrap_or(false)
    }

    fn read_ready(&self) -> FsResult<bool> {
        if !self.can_read() {
            return Err(FsError::PermissionDenied);
        }
        self.read_ready_file()
    }

    fn write_ready(&self) -> FsResult<bool> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.write_ready_file()
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.set_len_file(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.metadata_file()
    }
}

/// Observer for dynamic WASI fd lifecycle events.
///
/// The observer is optional host state. It lets embedding runtimes mirror WASI
/// dynamic file descriptors into their own process model without making this
/// crate depend on that process model.
pub trait WasiFdObserver: fmt::Debug + Send + Sync {
    /// Returns whether `fd` is available for a mirrored regular-file open.
    #[must_use]
    fn file_fd_available(&self, _fd: WasiFd) -> bool {
        true
    }

    /// Called after a regular file is opened at a dynamic WASI fd.
    ///
    /// Returning an error rejects the open.
    fn file_opened(&self, fd: WasiFd, file: WasiFile, path: &NormalizedPath) -> Result<(), Errno>;

    /// Called when a mirrored regular file fd should be closed.
    ///
    /// During explicit `WasiCtx::fd_close`, returning an error rejects the
    /// close and keeps the WASI fd open. Drop-time cleanup ignores errors.
    fn fd_closed(&self, fd: WasiFd) -> Result<(), Errno>;
}

/// WASI host configuration backed by a Wanix namespace.
#[derive(Clone)]
pub struct WasiConfig {
    namespace: Namespace,
    stdio: BTreeMap<WasiFd, WasiFile>,
    preopens: Vec<Preopen>,
    args: Vec<String>,
    env: Vec<String>,
    clock_time_ns: u64,
    fd_observer: Option<Arc<dyn WasiFdObserver>>,
}

impl fmt::Debug for WasiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasiConfig")
            .field("namespace", &self.namespace)
            .field("stdio_fds", &self.stdio.keys().collect::<Vec<_>>())
            .field("preopens", &self.preopens)
            .field("arg_count", &self.args.len())
            .field("env_count", &self.env.len())
            .field("clock_time_ns", &self.clock_time_ns)
            .field("has_fd_observer", &self.fd_observer.is_some())
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
            preopens: vec![Preopen::new(".").expect("root path is valid")],
            args: Vec::new(),
            env: Vec::new(),
            clock_time_ns: DEFAULT_CLOCK_TIME_NS,
            fd_observer: None,
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

    /// Returns configured process arguments in WASI argv order.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns configured environment strings in `KEY=value` form.
    #[must_use]
    pub fn env(&self) -> &[String] {
        &self.env
    }

    /// Returns the nanosecond timestamp used for WASI `*_NOW` timestamp flags.
    #[must_use]
    pub fn clock_time_ns(&self) -> u64 {
        self.clock_time_ns
    }

    pub(crate) fn stdio(&self) -> &BTreeMap<WasiFd, WasiFile> {
        &self.stdio
    }

    pub(crate) fn fd_observer(&self) -> Option<Arc<dyn WasiFdObserver>> {
        self.fd_observer.as_ref().map(Arc::clone)
    }

    /// Attaches an observer for dynamic regular-file fd lifecycle events.
    #[must_use]
    pub fn with_fd_observer(mut self, observer: impl WasiFdObserver + 'static) -> Self {
        self.fd_observer = Some(Arc::new(observer));
        self
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

    /// Replaces the configured process arguments in WASI argv order.
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the configured environment strings in `KEY=value` form.
    #[must_use]
    pub fn with_env<I, S>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.env = env.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the nanosecond timestamp used for WASI `*_NOW` timestamp flags.
    #[must_use]
    pub fn with_clock_time_ns(mut self, clock_time_ns: u64) -> Self {
        self.clock_time_ns = clock_time_ns;
        self
    }

    /// Replaces the fd 3 root preopen source while keeping its guest name as `/`.
    #[must_use]
    pub fn with_root_preopen_source(mut self, source_path: NormalizedPath) -> Self {
        self.preopens[0] = Preopen {
            source_path,
            guest_path: NormalizedPath::new(".").expect("root path is valid"),
        };
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
