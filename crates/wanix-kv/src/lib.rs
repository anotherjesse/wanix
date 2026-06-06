//! Key/value device filesystem for Rust Wanix.
//!
//! `KvDevice` exposes a `#kv` service where each key is a file: reading
//! `#kv/<key>` returns its value, writing it sets the value, removing it
//! deletes the key, and listing `#kv` enumerates the keys. It is the smallest
//! "real database inside Wanix" — an in-memory tier; durable and
//! content-addressed backing is a follow-up.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

mod files;

use files::{KvReadFile, KvWriteFile};

pub(crate) type Store = Arc<RwLock<BTreeMap<String, Vec<u8>>>>;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix key/value device filesystem";

pub(crate) mod modes {
    pub(crate) const VALUE_FILE: u32 = 0o666;
    pub(crate) const DIRECTORY: u32 = 0o555;
}

/// Filesystem implementing the Rust-native Wanix key/value service.
#[derive(Clone)]
pub struct KvDevice {
    store: Store,
}

impl fmt::Debug for KvDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.store.read() {
            Ok(store) => f
                .debug_struct("KvDevice")
                .field("key_count", &store.len())
                .finish(),
            Err(_) => f
                .debug_struct("KvDevice")
                .field("store", &"poisoned")
                .finish(),
        }
    }
}

impl Default for KvDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl KvDevice {
    /// Creates an empty key/value device.
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    fn store(&self) -> Store {
        Arc::clone(&self.store)
    }

    fn snapshot(&self, key: &str) -> FsResult<Vec<u8>> {
        let store = self
            .store
            .read()
            .map_err(|_| FsError::Other("kv device lock poisoned".to_owned()))?;
        store.get(key).cloned().ok_or(FsError::NotFound)
    }

    fn value_len(&self, key: &str) -> FsResult<u64> {
        let store = self
            .store
            .read()
            .map_err(|_| FsError::Other("kv device lock poisoned".to_owned()))?;
        store
            .get(key)
            .map(|value| value.len() as u64)
            .ok_or(FsError::NotFound)
    }

    /// Ensures a key exists (with an empty value if absent) so that a stat
    /// issued between open and the close-time commit succeeds — the 9P
    /// `Tlcreate` flow stats the path immediately after open.
    fn ensure_key(&self, key: &str) -> FsResult<()> {
        let mut store = self
            .store
            .write()
            .map_err(|_| FsError::Other("kv device lock poisoned".to_owned()))?;
        store.entry(key.to_owned()).or_default();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KvPath<'a> {
    Root,
    Key(&'a str),
}

fn parse_path(path: &NormalizedPath) -> FsResult<KvPath<'_>> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(KvPath::Root);
    }
    if raw.contains('/') {
        return Err(FsError::NotFound);
    }
    Ok(KvPath::Key(raw))
}

impl FileSystem for KvDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let KvPath::Key(key) = parse_path(path)? else {
            return Err(FsError::IsDirectory);
        };
        if options.write || options.create || options.truncate {
            self.ensure_key(key)?;
            return Ok(Box::new(KvWriteFile::new(self.store(), key.to_owned())));
        }
        if options.read {
            return Ok(Box::new(KvReadFile::new(self.snapshot(key)?)));
        }
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            KvPath::Root => Ok(directory_metadata()),
            KvPath::Key(key) => Ok(file_metadata(self.value_len(key)?)),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let KvPath::Root = parse_path(path)? else {
            return Err(FsError::NotDirectory);
        };
        let store = self
            .store
            .read()
            .map_err(|_| FsError::Other("kv device lock poisoned".to_owned()))?;
        Ok(store
            .iter()
            .map(|(key, value)| DirEntry::new(key.clone(), file_metadata(value.len() as u64)))
            .collect())
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let KvPath::Key(key) = parse_path(path)? else {
            return Err(FsError::IsDirectory);
        };
        let mut store = self
            .store
            .write()
            .map_err(|_| FsError::Other("kv device lock poisoned".to_owned()))?;
        store.remove(key).map(|_| ()).ok_or(FsError::NotFound)
    }
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

fn file_metadata(len: u64) -> Metadata {
    Metadata::new(FileType::File, len, modes::VALUE_FILE)
}

#[cfg(test)]
mod tests;
