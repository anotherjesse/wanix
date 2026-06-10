//! The open-file handles an AppFS view hands out.
//!
//! [`GuestFile`] routes discrete reads/writes to the guest. Reads are ranged
//! (v0.2): each guest fetch asks for one [`READ_CHUNK_LEN`] chunk at the
//! current offset and serves it slice-by-slice, so app files are not capped
//! by the wire's line ceiling — and a file smaller than one chunk still costs
//! exactly one guest fetch (the whole-snapshot path). A reply shorter than
//! the requested length marks end-of-file. Each write is one event.
//! [`StreamFile`] is host-owned: it drains its subscription's bounded lossy
//! [`LineBuffer`] and never consults the guest, so a wedged guest cannot
//! block it. [`BytesFile`] serves a fixed snapshot (`who`).

use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::fs::{file_metadata, modes};
use crate::protocol::{AppOp, READ_CHUNK_LEN, encode_data};
use crate::service::Shared;
use wanix_fs::LineBuffer;

/// Rejects any open that is not strictly read-only.
pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read || options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

/// A fixed byte snapshot served slice-by-slice (the `who` presence file).
pub(crate) struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
    mode: u32,
}

impl BytesFile {
    pub(crate) fn new(bytes: Vec<u8>, mode: u32) -> Self {
        Self {
            bytes,
            offset: 0,
            mode,
        }
    }
}

fn read_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> usize {
    let remaining = &bytes[(*offset).min(bytes.len())..];
    let len = remaining.len().min(buf.len());
    buf[..len].copy_from_slice(&remaining[..len]);
    *offset += len;
    len
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        Ok(read_slice(&self.bytes, &mut self.offset, buf))
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(self.bytes.len() as u64, self.mode))
    }
}

/// A guest-handled file: every discrete operation is one channel event.
pub(crate) struct GuestFile {
    shared: Arc<Shared>,
    path: String,
    principal: String,
    options: OpenOptions,
    /// The last fetched chunk, serving bytes `[chunk_start, chunk_start +
    /// chunk.len())`.
    chunk: Vec<u8>,
    chunk_start: u64,
    /// The byte position of the next read.
    offset: u64,
    /// The file length once a short chunk reply has revealed end-of-file.
    eof_at: Option<u64>,
}

impl GuestFile {
    pub(crate) fn new(
        shared: Arc<Shared>,
        path: String,
        principal: String,
        options: OpenOptions,
    ) -> Self {
        Self {
            shared,
            path,
            principal,
            options,
            chunk: Vec::new(),
            chunk_start: 0,
            offset: 0,
            eof_at: None,
        }
    }

    /// Serves `buf` from the cached chunk when it covers the current offset.
    fn read_cached(&mut self, buf: &mut [u8]) -> Option<usize> {
        let into_chunk = self.offset.checked_sub(self.chunk_start)?;
        let into_chunk = usize::try_from(into_chunk).ok()?;
        if into_chunk >= self.chunk.len() {
            return None;
        }
        let len = (self.chunk.len() - into_chunk).min(buf.len());
        buf[..len].copy_from_slice(&self.chunk[into_chunk..into_chunk + len]);
        self.offset += len as u64;
        Some(len)
    }
}

impl File for GuestFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.options.read {
            return Err(FsError::PermissionDenied);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(len) = self.read_cached(buf) {
            return Ok(len);
        }
        if self.eof_at.is_some_and(|eof| self.offset >= eof) {
            return Ok(0);
        }
        let ok = self.shared.transact(
            AppOp::Read,
            &self.path,
            &self.principal,
            None,
            Some((self.offset, READ_CHUNK_LEN)),
        )?;
        let bytes = ok.data_bytes()?;
        if bytes.len() as u64 > READ_CHUNK_LEN {
            return Err(FsError::Other(format!(
                "app guest replied {} bytes to a {READ_CHUNK_LEN}-byte ranged read",
                bytes.len()
            )));
        }
        if (bytes.len() as u64) < READ_CHUNK_LEN {
            self.eof_at = Some(self.offset + bytes.len() as u64);
        }
        self.chunk_start = self.offset;
        self.chunk = bytes;
        Ok(self.read_cached(buf).unwrap_or(0))
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.options.write {
            return Err(FsError::PermissionDenied);
        }
        // One write is one event: the guest sees the whole body or nothing,
        // so a partial count has no meaning (the plumb `send` discipline).
        self.shared.transact(
            AppOp::Write,
            &self.path,
            &self.principal,
            Some(encode_data(buf)),
            None,
        )?;
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        // One stat event: the guest may declare the file's size (v0.2);
        // without a declaration the size is honestly unknown and reads 0.
        let ok = self
            .shared
            .transact(AppOp::Stat, &self.path, &self.principal, None, None)?;
        Ok(file_metadata(ok.size.unwrap_or(0), modes::GUEST_FILE))
    }
}

/// A host-owned never-EOF stream subscription.
///
/// Reads drain the subscription's bounded lossy buffer and block while it is
/// empty; they never touch the guest channel. Dropping the handle removes the
/// subscription from the registry (and from `who`).
pub(crate) struct StreamFile {
    shared: Arc<Shared>,
    subscription: u64,
    buffer: Arc<LineBuffer>,
}

impl StreamFile {
    pub(crate) fn new(shared: Arc<Shared>, subscription: u64, buffer: Arc<LineBuffer>) -> Self {
        Self {
            shared,
            subscription,
            buffer,
        }
    }
}

impl File for StreamFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.buffer.read(buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.buffer.read_ready()
    }

    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

impl Drop for StreamFile {
    fn drop(&mut self) {
        self.shared.unsubscribe(self.subscription);
    }
}
