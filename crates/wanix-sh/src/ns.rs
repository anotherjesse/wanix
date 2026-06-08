//! The shell's only operating-system surface.
//!
//! [`NamespaceOps`] is the single seam between the pure shell logic and the
//! outside world. The wasm guest backs it with WASI over the task's Wanix
//! namespace (the `#task` device for launching children); host unit tests back
//! it with an in-memory fake. Keeping every side effect behind this trait is
//! what lets the parser, plan lowering, and executor be tested on the host with
//! no real I/O.

use crate::error::ShellResult;

/// A request to launch an external command as a child task.
///
/// The launcher (the WASI backing) resolves [`program`](Self::program) to a task
/// kind via the `#task` device (a `.wasm` program runs under the wasm driver, a
/// `.js` program under the qjs driver, …) and runs it to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The program to run (`argv[0]`).
    pub program: String,
    /// The arguments following the program name.
    pub args: Vec<String>,
}

impl SpawnSpec {
    /// Builds a spec from a resolved argument vector (`argv[0]` is the program).
    #[must_use]
    pub fn from_argv(argv: &[String]) -> Self {
        Self {
            program: argv.first().cloned().unwrap_or_default(),
            args: argv.get(1..).map(<[String]>::to_vec).unwrap_or_default(),
        }
    }
}

/// The host operations the shell needs to run a command line.
///
/// The surface grows one capability at a time as the executor gains features
/// (pipes, redirects, completion, …). Today it carries standard output/error and
/// external command launch with inherited stdio.
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

    /// Launches an external command, waits for it, and returns its exit code.
    ///
    /// Standard input/output/error are inherited from the shell. (Redirection
    /// and pipelines arrive in later phases.)
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be launched (e.g. the program is
    /// not found or a `#task` operation fails). A command that runs but exits
    /// non-zero returns `Ok(code)`.
    fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32>;
}
