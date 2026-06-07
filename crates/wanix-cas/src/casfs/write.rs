//! The write handle that keeps [`super::CasFs`]'s content hash fresh.
//!
//! While this handle is open the path is marked as being written, so
//! `content_hash` returns `None` and the file is read inline (never offloaded
//! mid-write). On drop the handle re-reads the file's now-committed bytes
//! through the inner filesystem, ingests them into the store (so the new content
//! address is immediately available), and clears the write marker — the
//! "hash-on-close" half of the freshness guard.

use std::sync::Arc;

use wanix_fs::{File, FileSeekFrom, FileSystem, FsResult, Metadata, NormalizedPath, OpenOptions};

use super::OpenWriters;
use crate::{ContentStore, MAX_BLOB_SIZE};

/// A pass-through write handle that re-ingests its file on close.
pub(crate) struct CasWriteFile {
    inner: Box<dyn File>,
    path: NormalizedPath,
    fs: Arc<dyn FileSystem>,
    store: Arc<dyn ContentStore>,
    open_writers: OpenWriters,
}

impl CasWriteFile {
    pub(crate) fn new(
        inner: Box<dyn File>,
        path: NormalizedPath,
        fs: Arc<dyn FileSystem>,
        store: Arc<dyn ContentStore>,
        open_writers: OpenWriters,
    ) -> Self {
        Self {
            inner,
            path,
            fs,
            store,
            open_writers,
        }
    }
}

impl File for CasWriteFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.inner.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.inner.write(buf)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.inner.seek(from)
    }

    fn tell(&self) -> FsResult<u64> {
        self.inner.tell()
    }

    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        self.inner.set_len(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.inner.metadata()
    }
}

impl Drop for CasWriteFile {
    fn drop(&mut self) {
        // Re-ingest the committed bytes so the new content address is ready, then
        // clear the write marker. Both are best-effort: a failed ingest just
        // means the next `content_hash` re-reads and re-ingests on demand.
        //
        // The re-read is bounded by MAX_BLOB_SIZE: a file larger than the blob
        // cap is never offloadable (`put` would reject it anyway), so we stop
        // accumulating and skip the ingest the moment the bytes exceed the cap.
        // This keeps every close O(cap) in memory instead of re-reading a
        // multi-GB file whole on each handle drop.
        if let Ok(mut file) = self.fs.open(&self.path, OpenOptions::read()) {
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 8192];
            let mut over_cap = false;
            while let Ok(read) = file.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                if bytes.len().saturating_add(read) > MAX_BLOB_SIZE {
                    over_cap = true;
                    break;
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            if !over_cap {
                let _ = self.store.put(&bytes);
            }
        }
        if let Ok(mut set) = self.open_writers.lock() {
            set.remove(self.path.as_str());
        }
    }
}
