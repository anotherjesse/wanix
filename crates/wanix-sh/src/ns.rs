//! The shell's only operating-system surface.
//!
//! [`NamespaceOps`] is the single seam between the pure shell logic and the
//! outside world. The wasm guest backs it with WASI over the task's Wanix
//! namespace (the `#task` device for launching children, `#pipe` for byte
//! channels); host unit tests back it with an in-memory fake. Keeping every side
//! effect behind this trait is what lets the parser, plan lowering, and executor
//! be tested on the host with no real I/O.

use crate::error::ShellResult;

/// Where a command stage reads its standard input from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSource {
    /// Inherit the shell's own standard input (fd 0).
    Inherit,
    /// Read the named `#pipe` channel (the read end).
    Pipe(String),
}

/// Where a command stage writes its standard output to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSink {
    /// Inherit the shell's own standard output (fd 1).
    Inherit,
    /// Write the named `#pipe` channel (the write end).
    Pipe(String),
}

/// A request to launch an external command as a child task.
///
/// The launcher (the WASI backing) resolves [`program`](Self::program) to a task
/// kind via the `#task` device (a `.wasm` program runs under the wasm driver, a
/// `.js` program under the qjs driver, …), wires its stdio per
/// [`stdin`](Self::stdin) / [`stdout`](Self::stdout) (stderr is always
/// inherited), and runs it to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The program to run (`argv[0]`).
    pub program: String,
    /// The arguments following the program name.
    pub args: Vec<String>,
    /// Where the child reads standard input.
    pub stdin: InputSource,
    /// Where the child writes standard output.
    pub stdout: OutputSink,
}

/// The host operations the shell needs to run a command line.
///
/// The surface grows one capability at a time as the executor gains features
/// (redirects, completion, …). Today it carries standard output/error, `#pipe`
/// byte channels, and external command launch with stdio wiring.
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

    /// Allocates a new `#pipe` channel and returns its id.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel cannot be allocated.
    fn pipe_new(&mut self) -> ShellResult<String>;

    /// Reads a `#pipe` channel to end-of-stream (the read end).
    ///
    /// # Errors
    ///
    /// Returns an error if the channel cannot be read.
    fn pipe_read_all(&mut self, id: &str) -> ShellResult<Vec<u8>>;

    /// Writes all bytes to a `#pipe` channel and closes the write end.
    ///
    /// Closing the writer is what lets the reader observe EOF — a builtin
    /// producer must release its end (the Plan 9 "shell closes its ends" move).
    ///
    /// # Errors
    ///
    /// Returns an error if the channel cannot be written.
    fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()>;

    /// Launches an external command, waits for it, and returns its exit code.
    ///
    /// Standard error is inherited from the shell; standard input/output are
    /// wired per the spec.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be launched (e.g. the program is
    /// not found or a `#task` operation fails). A command that runs but exits
    /// non-zero returns `Ok(code)`.
    fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32>;
}
