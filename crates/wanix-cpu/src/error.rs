//! The CPU job error surface shared by the acceptor and caller.

use std::fmt;

/// A CPU job failure on either the acceptor or the caller side.
#[derive(Debug)]
pub enum CpuError {
    /// The export 9P session (reverse-exported world) failed.
    Export(String),
    /// Allocating, binding, configuring, or starting the task failed.
    Task(String),
    /// Reading or writing the control stream failed.
    Control(String),
    /// The control stream ended before a terminal [`crate::CpuEvent::Exit`].
    NoExit,
    /// The caller's control drain was cancelled.
    ///
    /// Cancellation stops draining the control stream; it does **not** abort the
    /// guest running on the acceptor (the task driver has no abort hook).
    Cancelled,
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Export(message) => write!(f, "cpu export session failed: {message}"),
            Self::Task(message) => write!(f, "cpu task failed: {message}"),
            Self::Control(message) => write!(f, "cpu control stream failed: {message}"),
            Self::NoExit => {
                write!(f, "cpu control stream ended before a terminal exit event")
            }
            Self::Cancelled => write!(f, "cpu control drain was cancelled"),
        }
    }
}

impl std::error::Error for CpuError {}

/// A result over [`CpuError`].
pub type CpuResult<T> = Result<T, CpuError>;
