use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

mod virtual_files;

pub(crate) use virtual_files::{MAX_VIRTUAL_FILE_PATH_BYTES, validate_virtual_path_components};

/// Deterministic host import settings attached to a runtime instance.
///
/// WebAssembly memory snapshots do not capture Rust host state. Supply this
/// config when creating or restoring a runtime to define the clock, random, and
/// timezone behavior that QuickJS observes through its current host imports,
/// plus stdio capture policy for WASI writes and engine-only read-only virtual
/// fixture files.
///
/// Use live WASI providers on create/restore options for runtime filesystem,
/// namespace, process, and fd semantics. The virtual-file surface is not Wanix
/// task namespace plumbing.
#[derive(Clone, PartialEq, Eq)]
pub struct QuickJsHostConfig {
    clock_time_ns: u64,
    random_byte: u8,
    timezone_offset_seconds: i32,
    capture_stdout: bool,
    capture_stderr: bool,
    stdout_capture_byte_limit: Option<usize>,
    stderr_capture_byte_limit: Option<usize>,
    read_only_virtual_files: BTreeMap<Vec<u8>, Arc<[u8]>>,
}

impl fmt::Debug for QuickJsHostConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickJsHostConfig")
            .field("clock_time_ns", &self.clock_time_ns)
            .field("random_byte", &self.random_byte)
            .field("timezone_offset_seconds", &self.timezone_offset_seconds)
            .field("capture_stdout", &self.capture_stdout)
            .field("capture_stderr", &self.capture_stderr)
            .field("stdout_capture_byte_limit", &self.stdout_capture_byte_limit)
            .field("stderr_capture_byte_limit", &self.stderr_capture_byte_limit)
            .field(
                "read_only_virtual_file_count",
                &self.read_only_virtual_files.len(),
            )
            .finish()
    }
}

impl Default for QuickJsHostConfig {
    fn default() -> Self {
        Self {
            clock_time_ns: 1_700_000_000_000_000_000,
            random_byte: 0x42,
            timezone_offset_seconds: 0,
            capture_stdout: false,
            capture_stderr: false,
            stdout_capture_byte_limit: None,
            stderr_capture_byte_limit: None,
            read_only_virtual_files: BTreeMap::new(),
        }
    }
}

impl QuickJsHostConfig {
    /// Returns the default deterministic host import configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the nanosecond timestamp used by WASI `clock_time_get`.
    #[must_use]
    pub fn clock_time_ns(&self) -> u64 {
        self.clock_time_ns
    }

    /// Returns the repeated byte used by WASI `random_get`.
    #[must_use]
    pub fn random_byte(&self) -> u8 {
        self.random_byte
    }

    /// Returns the timezone offset, in seconds east of UTC, used by QuickJS.
    #[must_use]
    pub fn timezone_offset_seconds(&self) -> i32 {
        self.timezone_offset_seconds
    }

    /// Returns whether WASI `fd_write` captures stdout into the runtime buffer.
    #[must_use]
    pub fn captures_stdout(&self) -> bool {
        self.capture_stdout
    }

    /// Returns whether WASI `fd_write` captures stderr into the runtime buffer.
    #[must_use]
    pub fn captures_stderr(&self) -> bool {
        self.capture_stderr
    }

    /// Returns the maximum retained stdout capture size, in bytes.
    ///
    /// `None` means captured stdout is unbounded except for allocation failure.
    #[must_use]
    pub fn stdout_capture_byte_limit(&self) -> Option<usize> {
        self.stdout_capture_byte_limit
    }

    /// Returns the maximum retained stderr capture size, in bytes.
    ///
    /// `None` means captured stderr is unbounded except for allocation failure.
    #[must_use]
    pub fn stderr_capture_byte_limit(&self) -> Option<usize> {
        self.stderr_capture_byte_limit
    }

    /// Sets the nanosecond timestamp used by WASI `clock_time_get`.
    #[must_use]
    pub fn with_clock_time_ns(mut self, clock_time_ns: u64) -> Self {
        self.clock_time_ns = clock_time_ns;
        self
    }

    pub(crate) fn set_clock_time_ns(&mut self, clock_time_ns: u64) {
        self.clock_time_ns = clock_time_ns;
    }

    /// Sets the repeated byte used by WASI `random_get`.
    #[must_use]
    pub fn with_random_byte(mut self, random_byte: u8) -> Self {
        self.random_byte = random_byte;
        self
    }

    /// Sets the timezone offset, in seconds east of UTC, used by QuickJS.
    #[must_use]
    pub fn with_timezone_offset_seconds(mut self, timezone_offset_seconds: i32) -> Self {
        self.timezone_offset_seconds = timezone_offset_seconds;
        self
    }

    /// Sets whether stdout writes are captured instead of written to process stdout.
    ///
    /// This does not change the stdout capture byte limit. By default, captured
    /// stdout is unbounded except for allocation failure.
    #[must_use]
    pub fn with_stdout_capture(mut self, capture_stdout: bool) -> Self {
        self.capture_stdout = capture_stdout;
        self
    }

    /// Sets whether stderr writes are captured instead of written to process stderr.
    ///
    /// This does not change the stderr capture byte limit. By default, captured
    /// stderr is unbounded except for allocation failure.
    #[must_use]
    pub fn with_stderr_capture(mut self, capture_stderr: bool) -> Self {
        self.capture_stderr = capture_stderr;
        self
    }

    /// Sets the maximum retained stdout capture size, in bytes.
    ///
    /// Use `None` for unbounded capture. This does not enable stdout capture by
    /// itself; use [`Self::with_limited_stdout_capture`] to enable and limit in
    /// one call.
    #[must_use]
    pub fn with_stdout_capture_byte_limit(mut self, byte_limit: Option<usize>) -> Self {
        self.stdout_capture_byte_limit = byte_limit;
        self
    }

    /// Sets the maximum retained stderr capture size, in bytes.
    ///
    /// Use `None` for unbounded capture. This does not enable stderr capture by
    /// itself; use [`Self::with_limited_stderr_capture`] to enable and limit in
    /// one call.
    #[must_use]
    pub fn with_stderr_capture_byte_limit(mut self, byte_limit: Option<usize>) -> Self {
        self.stderr_capture_byte_limit = byte_limit;
        self
    }

    /// Enables stdout capture with a maximum retained size in bytes.
    ///
    /// The limit applies to retained buffer bytes. A limit of `0` allows empty
    /// writes only. Calling
    /// [`QuickJsRuntime::take_captured_stdout`](crate::QuickJsRuntime::take_captured_stdout)
    /// clears the retained buffer and resets the byte count.
    #[must_use]
    pub fn with_limited_stdout_capture(mut self, byte_limit: usize) -> Self {
        self.capture_stdout = true;
        self.stdout_capture_byte_limit = Some(byte_limit);
        self
    }

    /// Enables stderr capture with a maximum retained size in bytes.
    ///
    /// The limit applies to retained buffer bytes. A limit of `0` allows empty
    /// writes only. Calling
    /// [`QuickJsRuntime::take_captured_stderr`](crate::QuickJsRuntime::take_captured_stderr)
    /// clears the retained buffer and resets the byte count.
    #[must_use]
    pub fn with_limited_stderr_capture(mut self, byte_limit: usize) -> Self {
        self.capture_stderr = true;
        self.stderr_capture_byte_limit = Some(byte_limit);
        self
    }

    /// Enables stdout and stderr capture with the same per-stream byte limit.
    ///
    /// The limit is applied independently to each retained stream buffer.
    #[must_use]
    pub fn with_limited_stdio_capture(mut self, byte_limit: usize) -> Self {
        self.capture_stdout = true;
        self.capture_stderr = true;
        self.stdout_capture_byte_limit = Some(byte_limit);
        self.stderr_capture_byte_limit = Some(byte_limit);
        self
    }
}
