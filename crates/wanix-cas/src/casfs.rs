//! [`CasFs`]: the content-addressing decorator that surfaces the only
//! 9P→blob offload hook.
//!
//! `CasFs` wraps an inner [`FileSystem`] and a [`ContentStore`]. Every read it
//! serves goes through unchanged; what it adds is [`FileSystem::content_hash`]:
//! a path whose file is large enough to be worth offloading reports the
//! [`ContentHash`] of its current bytes, so a CAS-aware 9P client can fetch the
//! blob from the data plane (BLAKE3-verified) instead of looping `Tread` over
//! the control plane.
//!
//! # Freshness without fid-mode scanning
//!
//! A stale hash is a correctness bug: a client must never fetch a blob that no
//! longer matches a file being written. The 9P server's `FidEntry` does not
//! record open mode, so the blueprint forbids scanning fids for writers.
//! Instead `CasFs` tracks, in a shared set, every path currently open for write;
//! [`FileSystem::content_hash`] returns `None` for any such path (and for any
//! file at or below the offload threshold), and the write handle re-ingests the
//! file's bytes and clears the marker on close. So between open-for-write and
//! close, the file simply has no offloadable hash and is read inline — never
//! offloaded mid-write.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    CONTENT_HASH_OFFLOAD_THRESHOLD, ContentHash, DirEntry, File, FileSystem, FsError, FsResult,
    Metadata, MetadataLookup, NormalizedPath, OpenOptions,
};

use crate::{ContentStore, MAX_BLOB_SIZE};

mod write;

use write::CasWriteFile;

/// Set of paths currently open for write, shared between the filesystem and its
/// live write handles.
pub(crate) type OpenWriters = Arc<Mutex<HashSet<String>>>;

/// A content-addressing decorator over an inner [`FileSystem`].
#[derive(Clone)]
pub struct CasFs {
    inner: Arc<dyn FileSystem>,
    store: Arc<dyn ContentStore>,
    open_writers: OpenWriters,
}

impl CasFs {
    /// Wraps `inner`, ingesting offloadable file bytes into `store`.
    #[must_use]
    pub fn new(inner: Arc<dyn FileSystem>, store: Arc<dyn ContentStore>) -> Self {
        Self {
            inner,
            store,
            open_writers: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Returns whether `path` is currently open for write.
    fn is_being_written(&self, path: &str) -> bool {
        self.open_writers
            .lock()
            .map(|set| set.contains(path))
            .unwrap_or(true)
    }

    /// Reads `path`'s full bytes through the inner filesystem, aborting as soon
    /// as the accumulated bytes would exceed [`MAX_BLOB_SIZE`].
    ///
    /// The early abort matters even though `content_hash` already short-circuits
    /// on `metadata.len()`: a host-backed file can grow between the stat and the
    /// read, and a streaming inner file may report a stale length. Capping the
    /// accumulation bounds the in-memory cost regardless, so a multi-GB inner
    /// file can never OOM the server through a content-hash lookup.
    fn read_full(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        let mut file = self.inner.open(path, OpenOptions::read())?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            if bytes.len().saturating_add(read) > MAX_BLOB_SIZE {
                return Err(FsError::NotSupported);
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        Ok(bytes)
    }
}

impl FileSystem for CasFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let handle = self.inner.open(path, options)?;
        if options.write || options.truncate {
            // Mark the path as being written so no stale hash is offloaded, and
            // hand back a handle that re-ingests and clears the marker on close.
            if let Ok(mut set) = self.open_writers.lock() {
                set.insert(path.as_str().to_owned());
            }
            return Ok(Box::new(CasWriteFile::new(
                handle,
                path.clone(),
                Arc::clone(&self.inner),
                Arc::clone(&self.store),
                Arc::clone(&self.open_writers),
            )));
        }
        Ok(handle)
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
        let metadata = self.inner.metadata(path)?;
        // Only regular files past the offload threshold are worth a blob
        // round-trip, and never a file mid-write (its hash would be stale).
        // A file larger than the blob cap is also not offloadable: `put` would
        // reject it, so short-circuit on the cheap stat rather than reading the
        // whole (multi-GB) file into memory only to throw it away.
        if !metadata.file_type().is_file_like()
            || metadata.len() <= CONTENT_HASH_OFFLOAD_THRESHOLD
            || metadata.len() > MAX_BLOB_SIZE as u64
            || self.is_being_written(path.as_str())
        {
            return Ok(None);
        }
        let bytes = self.read_full(path)?;
        let hash = self
            .store
            .put(&bytes)
            .map_err(|err| FsError::Other(err.to_string()))?;
        Ok(Some(hash))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.inner.metadata(path)
    }

    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        self.inner.metadata_with_lookup(path, lookup)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        self.inner.read_dir(path)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        self.inner.read_link(path)
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        self.inner.symlink(target, path)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.inner.create_dir(path)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.inner.remove_dir(path)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.inner.rename(old_path, new_path)
    }
}

/// Helper extension on [`wanix_fs::FileType`] used to gate offload to regular
/// files (directories and symlinks are never blob-offloaded).
trait FileTypeExt {
    fn is_file_like(&self) -> bool;
}

impl FileTypeExt for wanix_fs::FileType {
    fn is_file_like(&self) -> bool {
        matches!(self, wanix_fs::FileType::File)
    }
}

#[cfg(test)]
mod tests;
