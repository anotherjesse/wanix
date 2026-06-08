//! The shell's only operating-system surface.
//!
//! [`NamespaceOps`] is the single seam between the pure shell logic and the
//! outside world. The wasm guest backs it with WASI (`std::io` / `std::fs` over
//! the task's Wanix namespace); host unit tests back it with an in-memory fake.
//! Keeping every side effect behind this trait is what lets the parser, plan
//! lowering, and executor be tested on the host with no real I/O.

use crate::error::ShellResult;

/// The host operations the shell needs to run a command line.
///
/// The surface is intentionally small and grows one method at a time as the
/// executor gains capabilities (task launch, pipes, completion, …). For now it
/// only carries standard output and error.
pub trait NamespaceOps {
    /// Writes bytes to the shell's standard output (fd 1).
    ///
    /// # Errors
    ///
    /// Returns [`ShellError::Io`](crate::ShellError::Io) if the write fails.
    fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()>;

    /// Writes bytes to the shell's standard error (fd 2).
    ///
    /// # Errors
    ///
    /// Returns [`ShellError::Io`](crate::ShellError::Io) if the write fails.
    fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()>;
}
