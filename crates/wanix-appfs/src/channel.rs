//! [`AppChannel`]: the byte-channel seam between the adapter and the guest.
//!
//! The adapter is transport-agnostic: it speaks newline-JSON lines to
//! whatever carries them. The CLI wires this to a resident guest task's
//! `#pipe` ends later; tests use an in-process scripted channel. The trait is
//! deliberately synchronous and line-oriented — the adapter owns request
//! serialization (one request in flight at a time, under one lock), so an
//! implementation only moves one line at a time.

use std::io;

/// A synchronous, line-oriented byte channel to the guest app.
///
/// `send_line` carries one complete adapter→guest line *including* its
/// trailing `\n`. `recv_line` returns one complete guest→adapter line; a
/// trailing `\n` is optional (the adapter trims it before parsing).
/// `recv_line` blocks until a line is available — the adapter relies on this
/// for its reply wait, and a guest that never replies wedges only the
/// discrete-op path, never host-owned stream reads.
pub trait AppChannel: Send {
    /// Sends one newline-terminated line to the guest.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the channel is broken; the adapter surfaces
    /// it as [`wanix_fs::FsError::Unreachable`] (the app is down, not missing).
    fn send_line(&mut self, line: &[u8]) -> io::Result<()>;

    /// Receives one line from the guest, blocking until one arrives.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the channel is broken; the adapter surfaces
    /// it as [`wanix_fs::FsError::Unreachable`].
    fn recv_line(&mut self) -> io::Result<Vec<u8>>;
}
