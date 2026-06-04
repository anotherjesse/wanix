use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use wanix_fs::{File, FsResult, NormalizedPath};
use wanix_vfs::Namespace;

use crate::{Errno, WasiFd};

mod file;

pub use file::WasiFile;

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
