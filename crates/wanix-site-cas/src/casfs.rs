//! [`CasSiteFs`]: a read-only [`FileSystem`] over a frozen site's root hash.
//!
//! Given a [`ContentStore`] and a [`CasRootHash`], `CasSiteFs` fetches and
//! parses the manifest blob once at open time, then serves every path by
//! resolving it against that manifest: a file path fetches its blob by hash, a
//! directory path is synthesized from the manifest's path prefixes (see
//! [`crate::tree`]), and every mutation is refused. It composes with the Phase 0
//! FS-backed static handler directly, so serving a `#cas` site is the same code
//! path as serving any other filesystem.

use std::sync::Arc;

use wanix_cas::{ContentStore, WorldManifest};
use wanix_fs::{
    ContentHash, DirEntry, File, FileSystem, FsError, FsResult, Metadata, NormalizedPath,
    OpenOptions,
};

use crate::file::CasBytesFile;
use crate::tree::{self, Resolved};
use crate::{CasRootHash, LoadError, manifest_entries};

/// A read-only filesystem view of a content-addressed site snapshot.
#[derive(Clone)]
pub struct CasSiteFs {
    store: Arc<dyn ContentStore>,
    manifest: WorldManifest,
    root: CasRootHash,
}

impl std::fmt::Debug for CasSiteFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CasSiteFs")
            .field("root", &self.root.to_hex())
            .field("file_count", &self.manifest.len())
            .finish()
    }
}

impl CasSiteFs {
    /// Loads the site at `root` from `store`, fetching and parsing its manifest.
    ///
    /// The manifest blob is fetched (BLAKE3-verified by the store), size-capped,
    /// and parsed with full path-safety + fan-out validation before the
    /// filesystem is returned, so a hostile root hash yields an error rather than
    /// an unsafe view.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Cas`] when the manifest blob is missing/corrupt and
    /// [`LoadError::Manifest`] when it cannot be parsed or violates a cap.
    pub fn open_root(store: Arc<dyn ContentStore>, root: CasRootHash) -> Result<Self, LoadError> {
        let blob = store.get(&root.hash()).map_err(LoadError::Cas)?;
        let manifest = WorldManifest::from_blob(&blob).map_err(LoadError::Manifest)?;
        Ok(Self {
            store,
            manifest,
            root,
        })
    }

    /// Returns the root hash this filesystem serves.
    #[must_use]
    pub fn root(&self) -> CasRootHash {
        self.root
    }

    /// Returns the number of files in the frozen site.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.manifest.len()
    }

    /// Fetches the blob for a resolved file entry by hash.
    fn fetch(&self, hash: &ContentHash) -> FsResult<Vec<u8>> {
        self.store
            .get(hash)
            .map_err(|err| FsError::Other(err.to_string()))
    }
}

impl FileSystem for CasSiteFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        if options.write || options.create || options.truncate {
            // A frozen site is immutable: writes are refused.
            return Err(FsError::PermissionDenied);
        }
        match tree::resolve(manifest_entries(&self.manifest), path.as_str()) {
            Resolved::File(entry) => {
                let bytes = self.fetch(&entry.hash)?;
                Ok(Box::new(CasBytesFile::new(bytes, tree::FILE_MODE)))
            }
            Resolved::Directory => Err(FsError::IsDirectory),
            Resolved::Missing => Err(FsError::NotFound),
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match tree::resolve(manifest_entries(&self.manifest), path.as_str()) {
            Resolved::File(entry) => {
                let len = self.fetch(&entry.hash)?.len() as u64;
                Ok(tree::file_metadata(len))
            }
            Resolved::Directory => Ok(tree::directory_metadata()),
            Resolved::Missing => Err(FsError::NotFound),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match tree::resolve(manifest_entries(&self.manifest), path.as_str()) {
            Resolved::Directory => Ok(tree::read_dir(
                manifest_entries(&self.manifest),
                path.as_str(),
            )),
            Resolved::File(_) => Err(FsError::NotDirectory),
            Resolved::Missing => Err(FsError::NotFound),
        }
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
        // A frozen site is content-addressed by construction: every file already
        // names its blob in the manifest, so the offload hook returns the entry
        // hash directly — a CAS-aware client fetches the blob peer-to-peer
        // instead of crawling the bytes through `Tread`. Directories have no hash.
        match tree::resolve(manifest_entries(&self.manifest), path.as_str()) {
            Resolved::File(entry) => Ok(Some(entry.hash)),
            Resolved::Directory => Ok(None),
            Resolved::Missing => Err(FsError::NotFound),
        }
    }
}
