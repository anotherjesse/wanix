//! The open-file handles a `#plumb` topic exposes: `send` and `recv`.
//!
//! `send` is write-only: each write is parsed as one JSON envelope and published
//! to the topic through the backing [`PlumbPort`]. `recv` is read-only and
//! blocking: it drains the newline-JSON envelope stream the subscription
//! delivers. The two are deliberately unidirectional so a topic directory reads
//! like Plan 9's `data`/`event` plumber files.

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::envelope::PlumbEnvelope;
use crate::port::{PlumbStream, SharedPlumbPort};
use crate::{file_metadata, modes};

/// Rejects any open that is not strictly write-only (for `send`).
pub(crate) fn require_write_only(options: OpenOptions) -> FsResult<()> {
    if !options.write || options.read {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

/// Rejects any open that is not strictly read-only (for `recv`).
pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read || options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

/// `#plumb/<topic>/send`: each write publishes one JSON envelope to the topic.
pub(crate) struct SendFile {
    port: SharedPlumbPort,
    topic: String,
}

impl SendFile {
    pub(crate) fn new(port: SharedPlumbPort, topic: String) -> Self {
        Self { port, topic }
    }
}

impl File for SendFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        // `send` is write-only; the open path already rejected a read open, but a
        // belt-and-braces guard keeps the contract explicit.
        Err(FsError::PermissionDenied)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        // One write is one envelope. The trailing newline a caller may include is
        // tolerated by the JSON parser, and the published line always carries its
        // own newline regardless.
        let envelope = PlumbEnvelope::parse(buf).map_err(FsError::Other)?;
        let line = envelope.to_line().map_err(FsError::Other)?;
        self.port.publish(&self.topic, &line)?;
        // Report the whole write consumed: a partial publish has no meaning on a
        // best-effort bus, so the unit of a write is the whole envelope.
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

/// `#plumb/<topic>/recv`: a blocking reader over the topic's received envelopes.
pub(crate) struct RecvFile {
    stream: Box<dyn PlumbStream>,
}

impl RecvFile {
    pub(crate) fn new(stream: Box<dyn PlumbStream>) -> Self {
        Self { stream }
    }
}

impl File for RecvFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.stream.read(buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.stream.read_ready()
    }

    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}
