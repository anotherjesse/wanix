//! Output sinks for a WASI guest: host-process passthrough and in-memory capture.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileType, FsError, FsResult, Metadata};

/// A [`File`] that forwards writes to the host process stdout or stderr.
///
/// Hands an embedder the same "see the guest's output" default the CLI uses,
/// without wiring up a [`CaptureFile`] and reading it back.
struct HostStream {
    stderr: bool,
}

impl File for HostStream {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        let written = if self.stderr {
            std::io::stderr().write(buf)
        } else {
            std::io::stdout().write(buf)
        };
        written.map_err(|e| FsError::Other(e.to_string()))
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o644))
    }
}

/// A stdout sink that forwards to the host process stdout.
#[must_use]
pub fn host_stdout() -> Box<dyn File> {
    Box::new(HostStream { stderr: false })
}

/// A stderr sink that forwards to the host process stderr.
#[must_use]
pub fn host_stderr() -> Box<dyn File> {
    Box::new(HostStream { stderr: true })
}

/// An in-memory [`File`] that captures everything written to it.
///
/// Useful as a stdout/stderr sink so a host can read back what a guest printed.
#[derive(Clone, Default)]
pub struct CaptureFile {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CaptureFile {
    /// Creates an empty capture file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bytes written so far as a lossy UTF-8 string.
    #[must_use]
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("capture lock")).into_owned()
    }

    /// Returns the raw bytes written so far.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.buffer.lock().expect("capture lock").clone()
    }
}

impl File for CaptureFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer
            .lock()
            .expect("capture lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o644))
    }
}
