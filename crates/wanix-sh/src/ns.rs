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
    /// Read the named file (a `<` redirection).
    File(String),
}

/// Where a command stage writes its standard output to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSink {
    /// Inherit the shell's own standard output (fd 1).
    Inherit,
    /// Write the named `#pipe` channel (the write end).
    Pipe(String),
    /// Write the named file (a `>` or, when `append`, `>>` redirection).
    File {
        /// Target path.
        path: String,
        /// Append rather than truncate.
        append: bool,
    },
}

/// An opaque handle to an external command launched with
/// [`NamespaceOps::spawn_start`], redeemed for an exit status by
/// [`NamespaceOps::spawn_wait`]. The WASI backing carries the child task id;
/// fakes carry whatever they need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnHandle(String);

impl SpawnHandle {
    /// Wraps a backend-specific identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the backend-specific identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.0
    }
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
    /// Environment variables to give the child (the shell's exported env).
    pub env: Vec<(String, String)>,
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

    /// Reads bytes from the shell's standard input (fd 0), blocking until at
    /// least one byte is available.
    ///
    /// Returns `Ok(0)` only at end-of-stream. The interactive REPL is the
    /// consumer: when fd 0 is a `#term/<id>/program` stream the read parks
    /// until the terminal client sends bytes (the host's blocking-read
    /// contract), and a closed stream ends the session.
    ///
    /// # Errors
    ///
    /// Returns [`ShellError::Io`](crate::ShellError::Io) if the read fails.
    fn read_stdin(&mut self, buf: &mut [u8]) -> ShellResult<usize>;

    /// Reports whether a path exists in the namespace (file or directory).
    ///
    /// Returns `Ok(false)` when the path is simply absent — that is not an error.
    /// Used by command resolution to find a program in the search directories.
    ///
    /// # Errors
    ///
    /// Returns an error only if existence cannot be determined (e.g. an I/O
    /// failure distinct from "not found").
    fn exists(&self, path: &str) -> ShellResult<bool>;

    /// Reads a file's full contents (for a `<` redirection on a builtin).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>>;

    /// Writes bytes to a file, truncating unless `append` (for `>`/`>>` on a
    /// builtin).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    fn write_file(&mut self, path: &str, bytes: &[u8], append: bool) -> ShellResult<()>;

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

    /// Holds a `#pipe` channel's write end open until the matching
    /// [`Self::pipe_write_all_and_close`].
    ///
    /// A pipeline's builtin producer holds its write end *before* the
    /// consumers start (the Unix "create the pipe before forking" move) —
    /// otherwise a concurrent consumer could observe writers == 0 and read a
    /// premature EOF before the builtin gets around to writing.
    ///
    /// # Errors
    ///
    /// Returns an error if the write end cannot be opened.
    fn pipe_open_writer(&mut self, id: &str) -> ShellResult<()>;

    /// Marks a `#pipe` channel as having lost its reader by opening and
    /// immediately dropping a read end (the `EPIPE`/`SIGPIPE` analog).
    ///
    /// Used when the stage that was meant to drain a pipe never launched or
    /// was skipped by an abort: a producer blocked on (or later writing to)
    /// the bounded channel then observes a broken-pipe error instead of
    /// parking forever against a buffer nothing will ever drain.
    ///
    /// # Errors
    ///
    /// Returns an error if the pipe's read end cannot be opened.
    fn pipe_break_reader(&mut self, id: &str) -> ShellResult<()>;

    /// Writes all bytes to a `#pipe` channel and closes the write end
    /// (including one held by [`Self::pipe_open_writer`]).
    ///
    /// Closing the writer is what lets the reader observe EOF — a builtin
    /// producer must release its end (the Plan 9 "shell closes its ends" move).
    ///
    /// # Errors
    ///
    /// Returns an error if the channel cannot be written.
    fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()>;

    /// Launches an external command WITHOUT waiting for it.
    ///
    /// The child runs concurrently with the shell (on the host, each running
    /// command task gets its own thread per ADR 0010 — an executor detail the
    /// shell never sees). Standard error is inherited; standard input/output
    /// are wired per the spec. Redeem the handle with [`Self::spawn_wait`].
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be launched (e.g. the program is
    /// not found or a `#task` operation fails).
    fn spawn_start(&mut self, spec: &SpawnSpec) -> ShellResult<SpawnHandle>;

    /// Blocks until a launched command exits and returns its exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if the exit status cannot be observed.
    fn spawn_wait(&mut self, handle: &SpawnHandle) -> ShellResult<i32>;

    /// Launches an external command, waits for it, and returns its exit code.
    ///
    /// The synchronous composition of [`Self::spawn_start`] and
    /// [`Self::spawn_wait`], used for single (non-pipeline) commands.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be launched. A command that runs
    /// but exits non-zero returns `Ok(code)`.
    fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32> {
        let handle = self.spawn_start(spec)?;
        self.spawn_wait(&handle)
    }
}
