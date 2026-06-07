//! File handles backing the `#cas` device: a snapshot reader and the
//! ingest-on-close writer.

use std::mem;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileType, FsError, FsResult, Metadata};

use crate::{ContentStore, MAX_BLOB_SIZE};

const BLOB_MODE: u32 = 0o444;

/// A read handle that serves a fixed byte snapshot captured at open time.
///
/// Blob reads, presence probes, and `ingest` hash reads all use this: the bytes
/// are produced once (verified, in the blob case) so a concurrent change cannot
/// tear an in-flight read.
pub(crate) struct BytesReadFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl BytesReadFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for BytesReadFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        let len = remaining.min(buf.len());
        buf[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
        self.offset += len;
        Ok(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(
            FileType::File,
            self.bytes.len() as u64,
            BLOB_MODE,
        ))
    }
}

/// A write handle that buffers bytes and ingests them as a blob on close.
///
/// On drop the buffered bytes are `put` into the store and the resulting
/// lowercase-hex content address is published into the device's shared
/// `last_ingest` slot, which a subsequent read of `#cas/ingest` returns. This is
/// the "write-then-read-hash" contract: the writer learns the address of what it
/// stored without any out-of-band channel.
///
/// The buffer is capped at [`MAX_BLOB_SIZE`] *at write time*, not merely at
/// `put` time: the `#cas` device is a [`wanix_fs::FileSystem`] that imports over
/// the mesh, so a remote peer with write access to `#cas/ingest` could otherwise
/// stream unbounded bytes into host memory before the close-time `put` cap ever
/// fired. Once the cumulative bytes would exceed the cap the write is rejected
/// with [`FsError::NotSupported`] and the handle is poisoned so the close-time
/// `put` is skipped — a partial, over-cap ingest never masquerades as success.
pub(crate) struct IngestFile {
    store: Arc<dyn ContentStore>,
    last_ingest: Arc<Mutex<Option<String>>>,
    buffer: Vec<u8>,
    /// Set once a write would exceed [`MAX_BLOB_SIZE`]; suppresses the close-time
    /// `put` so a rejected over-cap stream cannot publish a truncated blob.
    over_cap: bool,
}

impl IngestFile {
    pub(crate) fn new(
        store: Arc<dyn ContentStore>,
        last_ingest: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            store,
            last_ingest,
            buffer: Vec::new(),
            over_cap: false,
        }
    }
}

impl File for IngestFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        // Reject before allocating: a remote ingest must not be able to grow the
        // host buffer past the blob cap. Saturating add so a pathological length
        // cannot wrap and slip under the ceiling.
        if self.buffer.len().saturating_add(buf.len()) > MAX_BLOB_SIZE {
            self.over_cap = true;
            return Err(FsError::NotSupported);
        }
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(
            FileType::File,
            self.buffer.len() as u64,
            0o644,
        ))
    }
}

impl Drop for IngestFile {
    fn drop(&mut self) {
        // A stream that already tripped the cap is discarded, not committed: the
        // buffer holds only a truncated prefix and must never publish as a blob.
        if self.over_cap {
            return;
        }
        let bytes = mem::take(&mut self.buffer);
        if let Ok(hash) = self.store.put(&bytes)
            && let Ok(mut slot) = self.last_ingest.lock()
        {
            *slot = Some(hash.to_hex());
        }
    }
}
