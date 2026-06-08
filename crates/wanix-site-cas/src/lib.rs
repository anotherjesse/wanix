//! Freeze a site `FileSystem` tree to a `#cas` root hash, then serve it back.
//!
//! Publishing in Wanix is a filesystem operation. This crate is the immutable
//! half of it:
//!
//! - [`freeze_fs`] walks a site [`FileSystem`] (a generator's in-memory output,
//!   a `LocalFs`, a mesh mount) and ingests it into a [`ContentStore`]: every
//!   file becomes a blob, the deterministic sorted [`WorldManifest`] is itself a
//!   blob, and *that manifest blob's hash is the [`CasRootHash`]*. It reuses
//!   `wanix-cas`'s `WorldManifest`/`ManifestEntry` wire form verbatim — the only
//!   difference from [`wanix_cas::Capsule::freeze`] is that bytes are sourced
//!   through the `FileSystem` trait instead of `std::fs`, so an in-memory site
//!   freezes with no host disk.
//! - [`CasSiteFs`] is the read-only [`FileSystem`] that resolves a path against
//!   a root hash (walk the manifest, fetch leaves by hash). It composes with the
//!   Phase 0 FS-backed static handler directly: serving a `#cas` site is the
//!   same code path as serving any other filesystem.
//!
//! Because content is hashed, a publish is an atomic name repoint and a rollback
//! is repointing to an earlier hash — no file copying, and the old hash keeps
//! serving the old bytes for as long as its blobs live in the store.
//!
//! This crate is synchronous and depends only on `wanix-fs` and `wanix-cas`: no
//! tokio, no iroh, no Wasmtime.

mod casfs;
mod file;
mod tree;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use wanix_cas::{
    ContentStore, MAX_MANIFEST_ENTRIES, ManifestEntry, MaterializeError, WorldManifest,
};
use wanix_fs::{ContentHash, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

pub use casfs::CasSiteFs;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix freeze a site filesystem to a #cas root hash and serve it";

/// Maximum bytes pulled per [`wanix_fs::File::read`] call while draining a file.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// The content-addressed root of a frozen site: the hash of its manifest blob.
///
/// A [`CasRootHash`] *is* the published name of a site snapshot. Two sites with
/// the same tree freeze to the same root hash (the manifest is sorted), so the
/// name is a pure function of content — "this exact site." Binding a host to a
/// root hash is the atomic deploy; binding it to an earlier root hash is the
/// rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CasRootHash(ContentHash);

impl CasRootHash {
    /// Wraps a manifest-blob hash as a site root hash.
    #[must_use]
    pub fn from_hash(hash: ContentHash) -> Self {
        Self(hash)
    }

    /// Returns the underlying content hash.
    #[must_use]
    pub fn hash(&self) -> ContentHash {
        self.0
    }

    /// Returns the root hash as a 64-character lowercase-hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Parses a 64-character lowercase-hex root hash, returning `None` when the
    /// string is not a valid content hash.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        ContentHash::from_hex(hex).ok().map(Self)
    }
}

/// The error surface of [`freeze_fs`].
#[derive(Debug)]
pub enum FreezeError {
    /// The tree carried more than [`MAX_MANIFEST_ENTRIES`] files.
    TooManyEntries(usize),
    /// A path under the site root was not a safe relative Wanix path.
    UnsafePath(String),
    /// A filesystem error while walking or reading the site tree.
    Fs(FsError),
    /// A content-store error while ingesting a file or manifest blob.
    Cas(wanix_cas::CasError),
}

impl std::fmt::Display for FreezeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyEntries(count) => {
                write!(f, "site has {count} files, exceeding the manifest cap")
            }
            Self::UnsafePath(path) => write!(f, "unsafe site path: {path}"),
            Self::Fs(err) => write!(f, "filesystem error: {err}"),
            Self::Cas(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for FreezeError {}

impl From<FsError> for FreezeError {
    fn from(err: FsError) -> Self {
        Self::Fs(err)
    }
}

impl From<wanix_cas::CasError> for FreezeError {
    fn from(err: wanix_cas::CasError) -> Self {
        Self::Cas(err)
    }
}

/// Freezes the site rooted at `root` in `fs` into `store`, returning the root
/// hash.
///
/// Every regular file is ingested as a blob keyed by its `root`-relative path,
/// the sorted [`WorldManifest`] is built, and the manifest blob is ingested; its
/// hash is the returned [`CasRootHash`]. Symbolic links are skipped (a frozen
/// site carries content, not link topology), exactly as
/// [`wanix_cas::Capsule::freeze`] does for the host-disk path. The same tree
/// always produces the same root hash.
///
/// `root` is `"."` for the whole filesystem, or a subtree path; it is never held
/// under any namespace lock by this function — it only reads through the
/// `FileSystem` trait.
///
/// # Errors
///
/// Returns [`FreezeError::TooManyEntries`] when the tree exceeds the manifest
/// fan-out cap, [`FreezeError::Fs`] on a read failure, [`FreezeError::UnsafePath`]
/// when a path component is not safe-relative, and [`FreezeError::Cas`] when a
/// file or the manifest blob cannot be stored.
pub fn freeze_fs(
    store: &dyn ContentStore,
    fs: &dyn FileSystem,
    root: &str,
) -> Result<CasRootHash, FreezeError> {
    let root = NormalizedPath::new(root).map_err(|_| FreezeError::UnsafePath(root.to_owned()))?;
    let mut entries = BTreeMap::new();
    collect_files(store, fs, &root, "", &mut entries)?;
    if entries.len() > MAX_MANIFEST_ENTRIES {
        return Err(FreezeError::TooManyEntries(entries.len()));
    }
    let manifest = manifest_from_entries(entries);
    let id = store.put(&manifest.to_blob())?;
    Ok(CasRootHash(id))
}

/// Recursively ingests files under `dir` into `store`, keyed by their path
/// relative to the freeze root (`rel_prefix` accumulates that relative path).
fn collect_files(
    store: &dyn ContentStore,
    fs: &dyn FileSystem,
    dir: &NormalizedPath,
    rel_prefix: &str,
    out: &mut BTreeMap<String, ContentHash>,
) -> Result<(), FreezeError> {
    for entry in fs.read_dir(dir)? {
        let name = entry.name();
        let rel = join_rel(rel_prefix, name);
        let child = child_path(dir, name)?;
        match entry.metadata().file_type() {
            // A frozen site carries content; link topology is not represented, so
            // a symlink is skipped rather than dereferenced (which could escape).
            FileType::Symlink => continue,
            FileType::Directory => collect_files(store, fs, &child, &rel, out)?,
            FileType::File => {
                let bytes = read_full(fs, &child)?;
                let hash = store.put(&bytes)?;
                out.insert(rel, hash);
            }
        }
    }
    Ok(())
}

/// Builds the manifest from the sorted path→hash map. The `BTreeMap` iteration
/// order is the manifest's canonical sort, so the blob (and root hash) are
/// deterministic.
fn manifest_from_entries(entries: BTreeMap<String, ContentHash>) -> WorldManifest {
    // `WorldManifest` keeps no public constructor for entries, but it round-trips
    // through its own blob form, which validates path-safety and the caps. Build
    // the blob directly from the sorted entries, then parse it back so the
    // manifest is the exact same wire form `wanix-cas` produces and consumes.
    let mut blob = Vec::new();
    for (path, hash) in &entries {
        blob.extend_from_slice(hash.to_hex().as_bytes());
        blob.push(b' ');
        blob.extend_from_slice(path.as_bytes());
        blob.push(b'\n');
    }
    // `from_blob` only fails on the caps / malformed lines, neither of which a
    // freshly serialized sorted map can hit (entry count was already checked),
    // so fall back to an empty manifest rather than panicking.
    WorldManifest::from_blob(&blob).unwrap_or_default()
}

/// Joins a relative prefix and a child name into a `/`-separated relative path.
fn join_rel(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Builds the in-filesystem path of a child entry under `dir`.
fn child_path(dir: &NormalizedPath, name: &str) -> Result<NormalizedPath, FreezeError> {
    let joined = match dir.as_str() {
        "." => name.to_owned(),
        other => format!("{other}/{name}"),
    };
    NormalizedPath::new(joined).map_err(|_| FreezeError::UnsafePath(name.to_owned()))
}

/// Reads `path`'s full bytes through `fs`, aborting if it grows past the blob
/// cap (the store's `put` would reject it anyway).
fn read_full(fs: &dyn FileSystem, path: &NormalizedPath) -> Result<Vec<u8>, FreezeError> {
    let mut file = fs.open(path, OpenOptions::read())?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > wanix_cas::MAX_BLOB_SIZE {
            return Err(FreezeError::Cas(wanix_cas::CasError::TooLarge {
                len: bytes.len() + read,
            }));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

/// The error surface of loading a [`CasSiteFs`] from a root hash.
#[derive(Debug)]
pub enum LoadError {
    /// The manifest blob was missing, oversized, or failed verification.
    Cas(wanix_cas::CasError),
    /// The manifest blob could not be parsed (or violated a cap / path-safety).
    Manifest(MaterializeError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cas(err) => write!(f, "{err}"),
            Self::Manifest(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// One resolved manifest entry, re-exported for the directory/file modules.
pub(crate) use wanix_cas::ManifestEntry as Entry;

/// Internal helper: borrow a manifest's entries as a slice.
pub(crate) fn manifest_entries(manifest: &WorldManifest) -> &[ManifestEntry] {
    manifest.entries()
}
