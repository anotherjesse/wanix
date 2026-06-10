//! [`AppSender`]/[`AppReceiver`]: the byte-channel seam between the adapter
//! and the guest.
//!
//! The adapter is transport-agnostic: it speaks newline-JSON lines to
//! whatever carries them. The CLI wires the two halves to a resident guest
//! task's `#pipe` stdio ends; tests use in-process scripted halves. The seam
//! is synchronous and line-oriented, and it is split because the directions
//! have different owners:
//!
//! - The **send half** lives inside [`crate::AppFsService`]; one discrete
//!   operation uses it at a time under the service's actor lock. Dropping the
//!   service drops the sender, which for a pipe-backed guest closes its stdin
//!   (EOF: the guest exits).
//! - The **receive half** is owned by the service's pump thread, the single
//!   reader of guest output. Replies route to the in-flight operation and
//!   `{"publish":{...}}` lines fan out to stream subscribers the moment they
//!   arrive, whether or not an operation is in flight — so the guest's output
//!   channel is always drained and a slow consumer can never wedge it.

use std::io;

/// The adapter→guest half: carries one complete request line at a time.
pub trait AppSender: Send {
    /// Sends one newline-terminated line to the guest. May block on guest
    /// backpressure; progress is guaranteed because the pump keeps draining
    /// the guest's output while this side waits.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the channel is broken; the adapter surfaces
    /// it as [`wanix_fs::FsError::Unreachable`] (the app is down, not missing).
    fn send_line(&mut self, line: &[u8]) -> io::Result<()>;
}

/// The guest→adapter half: yields guest lines, owned by the pump thread.
pub trait AppReceiver: Send {
    /// Receives one line from the guest, blocking until one arrives. A
    /// trailing `\n` is optional (the pump trims it before parsing).
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the channel is broken (e.g. the guest
    /// exited); the pump then latches the channel down so discrete ops fail
    /// as [`wanix_fs::FsError::Unreachable`] and stream readers observe EOF.
    fn recv_line(&mut self) -> io::Result<Vec<u8>>;
}
