//! [`NativeFile`]: an open file handle that owns its dedicated open-file stream.
//!
//! The handle runs the [`FileOp`]/[`FileReply`] sub-protocol on its own stream:
//! every method writes one [`FileOp`] and reads one [`FileReply`]. The stream is
//! held behind a `Mutex` so the handle is `Send` (the trait requires it) while
//! the sub-protocol stays strictly request/reply.
//!
//! The seek/tell/offset discipline is ported from
//! `crates/wanix-9p-client/src/file.rs`: only files the server reports `seekable`
//! at open carry a client-tracked offset and accept `seek`; a device/service
//! stream reports `is_seekable() == false` and refuses `seek`. Because the
//! native `Read`/`Write` ops carry no offset (the server reads sequentially from
//! the open handle's own cursor), `seek` is **server-authoritative**: the client
//! sends [`FileOp::Seek`] and stores the absolute offset the server returns, then
//! keeps `tell()` client-tracked by advancing it across reads and writes.
//!
//! [`Drop`] finishes the client send half so the server observes the half-close
//! and releases its reader — the native replacement for 9P `Tclunk`.

use std::sync::Mutex;

use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata};

use crate::Duplex;
use crate::client::{protocol_mismatch, transport};
use crate::frame::{MAX_FRAME_LEN, read_frame, write_frame};
use crate::proto::{FileOp, FileReply};
use crate::value::WireSeek;

/// An open file handle backed by one dedicated native-wire stream.
pub struct NativeFile {
    stream: Mutex<Box<dyn Duplex>>,
    seekable: bool,
    offset: u64,
}

impl NativeFile {
    /// Builds an open-file handle over an already-opened stream.
    pub(crate) fn new(stream: Box<dyn Duplex>, seekable: bool) -> Self {
        Self {
            stream: Mutex::new(stream),
            seekable,
            offset: 0,
        }
    }

    /// Sends one [`FileOp`] and reads one [`FileReply`] under the stream lock.
    fn round_trip(&self, op: &FileOp) -> FsResult<FileReply> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| FsError::Other("mesh: open-file stream poisoned".to_owned()))?;
        write_frame(&mut *stream, op, MAX_FRAME_LEN)
            .map_err(|err| transport(std::io::Error::other(err)))?;
        read_frame(&mut *stream, MAX_FRAME_LEN).map_err(|err| transport(std::io::Error::other(err)))
    }
}

impl File for NativeFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let max = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        match self.round_trip(&FileOp::Read { max })? {
            // A 0-length Chunk is regular-file EOF; Eof is a never-EOF device's
            // real close. Both surface as a 0-byte read to the caller.
            FileReply::Chunk(data) => {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                if self.seekable {
                    self.offset = self.offset.saturating_add(n as u64);
                }
                Ok(n)
            }
            FileReply::Eof => Ok(0),
            FileReply::Err(error) => Err(error.into()),
            _ => Err(protocol_mismatch("read")),
        }
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        match self.round_trip(&FileOp::Write(buf.to_vec()))? {
            FileReply::Wrote(written) => {
                let written = written as usize;
                if self.seekable {
                    self.offset = self.offset.saturating_add(written as u64);
                }
                Ok(written)
            }
            FileReply::Err(error) => Err(error.into()),
            _ => Err(protocol_mismatch("write")),
        }
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        if !self.seekable {
            return Err(FsError::NotSupported);
        }
        // Native Read/Write carry no offset, so the server holds the cursor; the
        // client repositions it server-authoritatively and stores the absolute
        // offset the server returns, keeping tell() client-tracked.
        match self.round_trip(&FileOp::Seek(WireSeek::from(from)))? {
            FileReply::Seeked(offset) => {
                self.offset = offset;
                Ok(offset)
            }
            FileReply::Err(error) => Err(error.into()),
            _ => Err(protocol_mismatch("seek")),
        }
    }

    fn tell(&self) -> FsResult<u64> {
        if self.seekable {
            Ok(self.offset)
        } else {
            Err(FsError::NotSupported)
        }
    }

    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        match self.round_trip(&FileOp::SetLen(len))? {
            FileReply::Ok => Ok(()),
            FileReply::Err(error) => Err(error.into()),
            _ => Err(protocol_mismatch("set_len")),
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        match self.round_trip(&FileOp::Stat)? {
            FileReply::Stat(metadata) => Ok(Metadata::from(metadata)),
            FileReply::Err(error) => Err(error.into()),
            _ => Err(protocol_mismatch("metadata")),
        }
    }
}

impl Drop for NativeFile {
    fn drop(&mut self) {
        // Finish the client send half so the server observes the half-close and
        // releases its reader (the native replacement for Tclunk). A best-effort
        // Close frame lets a server that is mid-loop return promptly; dropping the
        // stream afterward closes the send side regardless.
        if let Ok(mut stream) = self.stream.lock() {
            let _ = write_frame(&mut *stream, &FileOp::Close, MAX_FRAME_LEN);
        }
    }
}
