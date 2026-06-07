//! The caller: reverse-export a scoped namespace and drain the result.
//!
//! The caller dials node Y, opens the two streams, and on each one:
//!
//! - **export**: runs its *own* [`wanix_9p::P9Server`] over the scoped,
//!   read-only-by-default sub-namespace built by [`crate::ExportScope`]. This is
//!   the reverse leg of Plan 9 cpu: the *caller* serves files, the *acceptor*
//!   imports them as the task world. It is **not** the caller's whole host root.
//! - **control**: reads the [`crate::CpuEvent`] batch the acceptor writes after
//!   the job runs, dispatching stdout/stderr to a [`JobOutput`] sink and
//!   returning the terminal exit status.
//!
//! [`serve_export`] and [`drive_control`] each take one already-role-sorted
//! stream, so the transport layer owns role discrimination and this module stays
//! transport-agnostic.

use std::io::{Read, Write};
use std::sync::Arc;

use wanix_9p::P9Server;
use wanix_fs::FileSystem;

use crate::CpuEvent;
use crate::error::{CpuError, CpuResult};

/// A sink for a job's batched stdout and stderr as the control stream is drained.
///
/// The caller implements this to route captured output (to its own stdio, a
/// buffer, a log, …). It is called once per [`CpuEvent::Stdout`] /
/// [`CpuEvent::Stderr`] frame, in the order the acceptor wrote them.
pub trait JobOutput {
    /// Receives a chunk of the job's captured standard output.
    fn stdout(&mut self, bytes: &[u8]);
    /// Receives a chunk of the job's captured standard error.
    fn stderr(&mut self, bytes: &[u8]);
}

/// A [`JobOutput`] that accumulates stdout and stderr into two `Vec`s.
///
/// Convenient for tests and for callers that want the whole batch in memory
/// rather than streamed to a writer.
#[derive(Debug, Default)]
pub struct CollectedOutput {
    /// Accumulated standard-output bytes, in arrival order.
    pub stdout: Vec<u8>,
    /// Accumulated standard-error bytes, in arrival order.
    pub stderr: Vec<u8>,
}

impl JobOutput for CollectedOutput {
    fn stdout(&mut self, bytes: &[u8]) {
        self.stdout.extend_from_slice(bytes);
    }

    fn stderr(&mut self, bytes: &[u8]) {
        self.stderr.extend_from_slice(bytes);
    }
}

/// Serves `root` as 9P over the export stream until the stream is closed.
///
/// This is the caller's reverse `P9Server`. `root` must be a scoped sub-namespace
/// (see [`crate::ExportScope`]), never the caller's whole host root. The server
/// is the unchanged synchronous [`P9Server::serve_stream`]; the acceptor's
/// [`wanix_9p_client::RemoteFs`] drives it.
///
/// Serving blocks until the export stream is closed. The **caller owns the
/// export's lifetime through the control protocol**: it runs this on a thread,
/// drains the control stream with [`drive_control`] until the terminal
/// [`CpuEvent::Exit`], and then shuts down its export stream (via a separate
/// handle — `TcpStream::shutdown`, the iroh stream's own close, or dropping a
/// cloned handle), which makes this read EOF and return. Relying on the acceptor
/// to close first is a TCP-FIN race *and* impossible when the acceptor's task
/// table holds the world in a `#task` self-binding cycle; the control `Exit`
/// event is the real, protocol-level "job done" signal, so the caller closes the
/// export. A read interrupted by that shutdown is reported as a clean end, not an
/// error.
///
/// # Errors
///
/// Returns [`CpuError::Export`] when the 9P transport fails for a reason other
/// than a stream close / clean end-of-stream.
pub fn serve_export<S>(root: Arc<dyn FileSystem>, stream: S) -> CpuResult<()>
where
    S: Read + Write,
{
    let mut server = P9Server::new(root);
    // serve_stream reads and writes on the one bidi stream, strictly serially.
    let (reader, writer) = split_duplex(stream);
    match server.serve_stream(reader, writer) {
        Ok(_) | Err(wanix_9p::P9TransportError::Io(_)) => Ok(()),
        Err(err) => Err(CpuError::Export(err.to_string())),
    }
}

/// Drains the control stream, dispatching output and returning the exit status.
///
/// Reads [`CpuEvent`] frames until a terminal [`CpuEvent::Exit`] arrives (its
/// code is returned) or the stream ends. A [`CpuEvent::Cancel`] written *by the
/// caller's side* into this drain — or a caller that drops the control reader —
/// stops draining; it does **not** cancel the running guest on node Y, which has
/// no abort hook. That asymmetry is intentional and documented, not faked.
///
/// # Errors
///
/// Returns [`CpuError::Control`] when reading a frame fails, and
/// [`CpuError::NoExit`] when the stream ends before a terminal exit event.
pub fn drive_control<R: Read>(control: &mut R, output: &mut dyn JobOutput) -> CpuResult<i32> {
    loop {
        let event =
            CpuEvent::read_from(control).map_err(|err| CpuError::Control(err.to_string()))?;
        match event {
            Some(CpuEvent::Stdout(bytes)) => output.stdout(&bytes),
            Some(CpuEvent::Stderr(bytes)) => output.stderr(&bytes),
            Some(CpuEvent::Exit(code)) => return Ok(code),
            // Cancel stops draining (best-effort); the remote guest keeps running.
            Some(CpuEvent::Cancel) => return Err(CpuError::Cancelled),
            None => return Err(CpuError::NoExit),
        }
    }
}

/// Splits a duplex stream into independent read and write halves for the server.
///
/// `serve_stream` wants a separate `Read` and `Write`. A duplex `S: Read + Write`
/// is shared behind a cheap clone-free split: the two halves alias the same
/// stream, which is safe because `serve_stream` alternates strictly serially
/// between reading a request and writing its response.
fn split_duplex<S: Read + Write>(stream: S) -> (DuplexRead<S>, DuplexWrite<S>) {
    let shared = Arc::new(std::sync::Mutex::new(stream));
    (
        DuplexRead {
            shared: Arc::clone(&shared),
        },
        DuplexWrite { shared },
    )
}

/// The read half of a serially-shared duplex stream.
struct DuplexRead<S> {
    shared: Arc<std::sync::Mutex<S>>,
}

/// The write half of a serially-shared duplex stream.
struct DuplexWrite<S> {
    shared: Arc<std::sync::Mutex<S>>,
}

impl<S: Read> Read for DuplexRead<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut stream = self
            .shared
            .lock()
            .map_err(|_| std::io::Error::other("export stream lock poisoned"))?;
        stream.read(buf)
    }
}

impl<S: Write> Write for DuplexWrite<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut stream = self
            .shared
            .lock()
            .map_err(|_| std::io::Error::other("export stream lock poisoned"))?;
        stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut stream = self
            .shared
            .lock()
            .map_err(|_| std::io::Error::other("export stream lock poisoned"))?;
        stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_control_dispatches_then_returns_the_exit_code() {
        let mut buf = Vec::new();
        CpuEvent::Stdout(b"out".to_vec())
            .write_to(&mut buf)
            .unwrap();
        CpuEvent::Stderr(b"err".to_vec())
            .write_to(&mut buf)
            .unwrap();
        CpuEvent::Exit(3).write_to(&mut buf).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let mut collected = CollectedOutput::default();
        let code = drive_control(&mut cursor, &mut collected).unwrap();
        assert_eq!(code, 3);
        assert_eq!(collected.stdout, b"out");
        assert_eq!(collected.stderr, b"err");
    }

    #[test]
    fn drive_control_without_an_exit_is_an_error() {
        let mut buf = Vec::new();
        CpuEvent::Stdout(b"x".to_vec()).write_to(&mut buf).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let mut collected = CollectedOutput::default();
        assert!(matches!(
            drive_control(&mut cursor, &mut collected),
            Err(CpuError::NoExit)
        ));
    }

    #[test]
    fn a_cancel_event_stops_draining() {
        let mut buf = Vec::new();
        CpuEvent::Cancel.write_to(&mut buf).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let mut collected = CollectedOutput::default();
        assert!(matches!(
            drive_control(&mut cursor, &mut collected),
            Err(CpuError::Cancelled)
        ));
    }
}
