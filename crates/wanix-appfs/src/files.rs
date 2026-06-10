//! The open-file handles an AppFS view hands out.
//!
//! [`GuestFile`] routes discrete reads/writes to the guest (a read fetches
//! one whole-content snapshot lazily and serves it slice-by-slice; each write
//! is one event). [`StreamFile`] is host-owned: it drains its subscription's
//! bounded lossy [`LineBuffer`] and never consults the guest, so a wedged
//! guest cannot block it. [`BytesFile`] serves a fixed snapshot (`who`).

use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::buffer::LineBuffer;
use crate::fs::{file_metadata, modes};
use crate::protocol::{AppOp, encode_data};
use crate::service::Shared;

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
    /// Whole-content snapshot fetched on first read, then served by offset.
    content: Option<Vec<u8>>,
    offset: usize,
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
            content: None,
            offset: 0,
        }
    }
}

impl File for GuestFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.options.read {
            return Err(FsError::PermissionDenied);
        }
        if self.content.is_none() {
            let ok = self
                .shared
                .transact(AppOp::Read, &self.path, &self.principal, None)?;
            self.content = Some(ok.data_bytes()?);
        }
        let content = self.content.as_deref().unwrap_or_default();
        Ok(read_slice(content, &mut self.offset, buf))
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
        )?;
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::GUEST_FILE))
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
