//! The shell's error type.

use std::fmt;

/// An error produced while parsing or executing a shell command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    /// The input could not be tokenized or parsed as shell syntax.
    Parse(String),
    /// A construct parsed correctly but the executor does not implement it yet.
    ///
    /// This is the honest-scope path: `brush-parser` accepts the full bash
    /// grammar, but the Wanix executor supports an explicit subset. Anything
    /// outside that subset returns this error rather than silently doing nothing.
    Unsupported(String),
    /// An operation against the namespace / OS surface failed.
    Io(String),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellError::Parse(msg) => write!(f, "syntax error: {msg}"),
            ShellError::Unsupported(what) => write!(f, "{what} not supported yet"),
            ShellError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// The result type for shell operations.
pub type ShellResult<T> = Result<T, ShellError>;
