//! The `#cas` device filesystem: blobs operable as ordinary Wanix files.
//!
//! [`CasDevice`] wraps any [`ContentStore`] and exposes it as a namespace any
//! task can bind, so `#cas` imports across the mesh for free (it is just a
//! `FileSystem`, like `#kv`):
//!
//! - `#cas/<hash>` — read-only; reading returns the blob's bytes (verified by
//!   the store), `stat` reports its length. Writing is refused.
//! - `#cas/ingest` — write-then-read-hash. Writing bytes and closing the handle
//!   stores them as a blob; reading `#cas/ingest` afterward returns the
//!   lowercase-hex hash of the most recently ingested blob, so a client learns
//!   the content address of what it just wrote without a side channel.
//! - `#cas/have/<hash>` — read-only presence probe; reading returns `1\n` when
//!   the blob is present locally and `0\n` when it is not.
//!
//! The hash path components are validated through
//! [`wanix_fs::ContentHash::from_hex`], so a malformed or hostile `<hash>` is
//! rejected before any store lookup.

use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    ContentHash, DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath,
    OpenOptions,
};

use crate::ContentStore;

mod files;

use files::{BytesReadFile, IngestFile};

/// Directory name segment used to probe blob presence: `#cas/have/<hash>`.
const HAVE_DIR: &str = "have";
/// File name used to ingest a new blob: `#cas/ingest`.
const INGEST_FILE: &str = "ingest";

const DIRECTORY_MODE: u32 = 0o555;
const BLOB_MODE: u32 = 0o444;
const INGEST_MODE: u32 = 0o644;

/// A `#cas` service filesystem over a shared [`ContentStore`].
#[derive(Clone)]
pub struct CasDevice {
    store: Arc<dyn ContentStore>,
    /// Hex hash of the most recently ingested blob, surfaced by reading
    /// `#cas/ingest`. Shared so a write handle's close can publish the address
    /// that the next read of `ingest` returns.
    last_ingest: Arc<Mutex<Option<String>>>,
}

impl fmt::Debug for CasDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CasDevice").finish_non_exhaustive()
    }
}

impl CasDevice {
    /// Wraps `store` as a `#cas` device.
    #[must_use]
    pub fn new(store: Arc<dyn ContentStore>) -> Self {
        Self {
            store,
            last_ingest: Arc::new(Mutex::new(None)),
        }
    }
}

/// The parsed shape of a path under `#cas`.
enum CasPath {
    /// `.` — the `#cas` root directory.
    Root,
    /// `ingest` — the write-then-read-hash file.
    Ingest,
    /// `have` — the presence-probe directory.
    HaveDir,
    /// `have/<hash>` — a presence probe for one blob.
    Have(ContentHash),
    /// `<hash>` — a single read-only blob.
    Blob(ContentHash),
}

fn parse_path(path: &NormalizedPath) -> FsResult<CasPath> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(CasPath::Root);
    }
    if raw == INGEST_FILE {
        return Ok(CasPath::Ingest);
    }
    if raw == HAVE_DIR {
        return Ok(CasPath::HaveDir);
    }
    if let Some(hash) = raw.strip_prefix("have/") {
        return Ok(CasPath::Have(ContentHash::from_hex(hash)?));
    }
    if raw.contains('/') {
        return Err(FsError::NotFound);
    }
    Ok(CasPath::Blob(ContentHash::from_hex(raw)?))
}

impl CasDevice {
    fn blob_len(&self, hash: &ContentHash) -> FsResult<u64> {
        let bytes = self.store.get(hash).map_err(cas_to_fs)?;
        Ok(bytes.len() as u64)
    }

    fn have_byte(&self, hash: &ContentHash) -> FsResult<Vec<u8>> {
        let present = self.store.has(hash).map_err(cas_to_fs)?;
        Ok(if present {
            b"1\n".to_vec()
        } else {
            b"0\n".to_vec()
        })
    }
}

impl FileSystem for CasDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match parse_path(path)? {
            CasPath::Root | CasPath::HaveDir => Err(FsError::IsDirectory),
            CasPath::Ingest => {
                if options.write || options.create || options.truncate {
                    return Ok(Box::new(IngestFile::new(
                        Arc::clone(&self.store),
                        Arc::clone(&self.last_ingest),
                    )));
                }
                let hex = self
                    .last_ingest
                    .lock()
                    .map_err(|_| FsError::Other("cas ingest lock poisoned".to_owned()))?
                    .clone()
                    .unwrap_or_default();
                Ok(Box::new(BytesReadFile::new(hex.into_bytes())))
            }
            CasPath::Blob(hash) => {
                if options.write || options.create || options.truncate {
                    return Err(FsError::PermissionDenied);
                }
                let bytes = self.store.get(&hash).map_err(cas_to_fs)?;
                Ok(Box::new(BytesReadFile::new(bytes)))
            }
            CasPath::Have(hash) => {
                if options.write || options.create || options.truncate {
                    return Err(FsError::PermissionDenied);
                }
                Ok(Box::new(BytesReadFile::new(self.have_byte(&hash)?)))
            }
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            CasPath::Root | CasPath::HaveDir => Ok(dir_metadata()),
            CasPath::Ingest => Ok(file_metadata(INGEST_MODE, 0)),
            CasPath::Blob(hash) => Ok(file_metadata(BLOB_MODE, self.blob_len(&hash)?)),
            CasPath::Have(hash) => Ok(file_metadata(
                BLOB_MODE,
                self.have_byte(&hash)?.len() as u64,
            )),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        // The blob keyspace is unbounded and not enumerable by design (a CAS is
        // looked up by hash, never listed), so the root advertises only the
        // operable control files; `have/` is likewise probe-only.
        match parse_path(path)? {
            CasPath::Root => Ok(vec![
                DirEntry::new(INGEST_FILE, file_metadata(INGEST_MODE, 0)),
                DirEntry::new(HAVE_DIR, dir_metadata()),
            ]),
            CasPath::HaveDir => Ok(Vec::new()),
            _ => Err(FsError::NotDirectory),
        }
    }
}

/// Maps a [`CasError`](crate::CasError) into the filesystem error surface.
fn cas_to_fs(err: crate::CasError) -> FsError {
    match err {
        crate::CasError::NotFound => FsError::NotFound,
        crate::CasError::TooLarge { .. } => FsError::NotSupported,
        other => FsError::Other(other.to_string()),
    }
}

fn dir_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 0, DIRECTORY_MODE)
}

fn file_metadata(mode: u32, len: u64) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

#[cfg(test)]
mod tests;
