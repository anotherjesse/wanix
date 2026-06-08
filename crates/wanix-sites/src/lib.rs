//! The `#sites` device: bind a host to a filesystem source.
//!
//! Publishing a website is a filesystem operation in Wanix. `SitesDevice` is a
//! plain [`FileSystem`] (like `#kv`/`#pipe`): `#sites/<host>` binds a hostname
//! to a [`SiteSource`], listing `#sites` enumerates the bound hosts, reading a
//! host's control file shows its binding, and writing one creates or repoints
//! the binding. The serve HTTP gateway reads the request `Host` header and
//! serves the resolved filesystem, so a site addressed by `blog.localhost`
//! works with zero DNS configuration.
//!
//! A live in-memory source (a generator's output filesystem) cannot be created
//! by value over 9P, so it is registered in-process via [`SitesDevice::bind_site`];
//! the writable control file only sets a descriptor source (`memory`/`cas`).
//! Resolving a [`SiteSource::Cas`] is filled in by Phase 3 (CAS-backed serving).

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use wanix_cas::ContentStore;
use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, LocalFs, Metadata, NormalizedPath,
    OpenOptions,
};
use wanix_site_cas::{CasRootHash, CasSiteFs};

mod files;
mod host;

use files::{SitesReadFile, SitesWriteFile};
pub use host::Host;

pub(crate) type Bindings = Arc<RwLock<BTreeMap<Host, SiteSource>>>;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix host-to-filesystem site binding device";

pub(crate) mod modes {
    pub(crate) const CONTROL_FILE: u32 = 0o666;
    pub(crate) const DIRECTORY: u32 = 0o555;
}

/// What a host binding points at.
#[derive(Clone)]
pub enum SiteSource {
    /// A live filesystem: an in-memory generator output, a `LocalFs`, etc.
    Memory(Arc<dyn FileSystem>),
    /// A host directory, served through a `LocalFs`. Registered by a file write
    /// (`dir <path>` to `#sites/<host>`) so that exposing a directory as a site
    /// is a filesystem operation, not a CLI flag.
    Dir(PathBuf),
    /// An immutable CAS snapshot, named by its root hash (hex). Resolving this
    /// to a servable filesystem requires the device to be built with a backing
    /// content store ([`SitesDevice::with_store`]); it then loads a
    /// [`CasSiteFs`] over that root hash.
    Cas(String),
}

impl SiteSource {
    /// Renders the stable, greppable control-file descriptor for this source,
    /// e.g. `memory\n` or `cas <root-hash>\n`.
    #[must_use]
    pub fn descriptor(&self) -> String {
        match self {
            Self::Memory(_) => "memory\n".to_owned(),
            Self::Dir(path) => format!("dir {}\n", path.display()),
            Self::Cas(hash) => format!("cas {hash}\n"),
        }
    }

    /// Parses a descriptor written to a control file back into a source. A bare
    /// `memory` becomes a placeholder `Memory` over an empty filesystem (a live
    /// source must be registered in-process); `cas <hash>` becomes a CAS source.
    pub(crate) fn parse_descriptor(descriptor: &str) -> Option<Self> {
        let trimmed = descriptor.trim();
        if trimmed.eq_ignore_ascii_case("memory") {
            return Some(Self::Memory(Arc::new(EmptySiteFs)));
        }
        if let Some(path) = trimmed
            .strip_prefix("dir")
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            return Some(Self::Dir(PathBuf::from(path)));
        }
        let hash = trimmed
            .strip_prefix("cas")
            .map(str::trim)
            .filter(|hash| !hash.is_empty())?;
        Some(Self::Cas(hash.to_owned()))
    }
}

/// The error surface of [`SitesDevice::publish`].
#[derive(Debug)]
pub enum PublishError {
    /// The device was built without a content store, so nothing can be frozen.
    NoStore,
    /// Freezing the site filesystem failed.
    Freeze(wanix_site_cas::FreezeError),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStore => f.write_str("sites device has no backing content store"),
            Self::Freeze(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PublishError {}

impl From<wanix_site_cas::FreezeError> for PublishError {
    fn from(err: wanix_site_cas::FreezeError) -> Self {
        Self::Freeze(err)
    }
}

impl fmt::Debug for SiteSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Memory(..)"),
            Self::Dir(path) => write!(f, "Dir({})", path.display()),
            Self::Cas(hash) => write!(f, "Cas({hash})"),
        }
    }
}

/// The `#sites` device: a host→source binding table exposed as a filesystem.
#[derive(Clone)]
pub struct SitesDevice {
    bindings: Bindings,
    /// The content store that backs [`SiteSource::Cas`] resolution. `None` when
    /// the device was built without one ([`SitesDevice::new`]); a `Cas` source
    /// then records its binding but cannot be served.
    store: Option<Arc<dyn ContentStore>>,
}

impl fmt::Debug for SitesDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.bindings.read() {
            Ok(bindings) => f
                .debug_struct("SitesDevice")
                .field("site_count", &bindings.len())
                .finish(),
            Err(_) => f
                .debug_struct("SitesDevice")
                .field("bindings", &"poisoned")
                .finish(),
        }
    }
}

impl Default for SitesDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl SitesDevice {
    /// Creates an empty site-binding device with no CAS backing.
    ///
    /// A [`SiteSource::Cas`] binding registered on such a device resolves to
    /// `None` (it has no store to load blobs from); use [`Self::with_store`] to
    /// serve immutable CAS snapshots.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Arc::new(RwLock::new(BTreeMap::new())),
            store: None,
        }
    }

    /// Creates an empty site-binding device backed by `store`, so a
    /// [`SiteSource::Cas`] binding can be resolved to an immutable
    /// [`CasSiteFs`] over its root hash.
    ///
    /// Sharing the *same* store instance with the `#cas` device means a blob
    /// ingested through `#cas` (or written by a freeze) is readable by a site by
    /// hash — publishing is then a pure name repoint with no file copying.
    #[must_use]
    pub fn with_store(store: Arc<dyn ContentStore>) -> Self {
        Self {
            bindings: Arc::new(RwLock::new(BTreeMap::new())),
            store: Some(store),
        }
    }

    /// Registers or replaces a host's binding programmatically (serve startup,
    /// tests). This is the only way to install a live [`SiteSource::Memory`]
    /// source, since a filesystem cannot be created by value over 9P.
    pub fn bind_site(&self, host: Host, source: SiteSource) {
        if let Ok(mut bindings) = self.bindings.write() {
            bindings.insert(host, source);
        }
    }

    /// Freezes `site` (rooted at `root`, normally `"."`) into this device's
    /// content store and binds `host` to the resulting immutable snapshot,
    /// returning its root hash.
    ///
    /// This is the publish operation: freeze the current output → get a root
    /// hash → repoint the host binding to it. Because the prior binding's blobs
    /// stay in the store, an earlier root hash keeps serving its bytes, so a
    /// later [`bind_site`](Self::bind_site) to that hash is an instant rollback.
    /// The freeze reads `site` purely through the [`FileSystem`] trait and never
    /// holds the `#sites` binding lock.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError::NoStore`] when the device has no backing store
    /// and [`PublishError::Freeze`] when the site cannot be frozen.
    pub fn publish(
        &self,
        host: Host,
        site: &dyn FileSystem,
        root: &str,
    ) -> Result<CasRootHash, PublishError> {
        let store = self.store.clone().ok_or(PublishError::NoStore)?;
        let root_hash = wanix_site_cas::freeze_fs(store.as_ref(), site, root)?;
        self.bind_site(host, SiteSource::Cas(root_hash.to_hex()));
        Ok(root_hash)
    }

    /// Resolves a host to the filesystem to serve.
    ///
    /// The source is cloned out of the binding lock before any filesystem is
    /// built, so the caller never holds the `#sites` lock while reading (or, for
    /// a CAS source, while fetching and parsing the manifest blob). A
    /// [`SiteSource::Cas`] resolves to a read-only [`CasSiteFs`] when the device
    /// has a backing store and the root hash and manifest load; otherwise it
    /// resolves to `None`.
    #[must_use]
    pub fn resolve(&self, host: &Host) -> Option<Arc<dyn FileSystem>> {
        let source = {
            let bindings = self.bindings.read().ok()?;
            bindings.get(host).cloned()?
        };
        match source {
            SiteSource::Memory(fs) => Some(fs),
            SiteSource::Dir(path) => LocalFs::new(&path)
                .ok()
                .map(|fs| Arc::new(fs) as Arc<dyn FileSystem>),
            SiteSource::Cas(hash) => self.resolve_cas(&hash),
        }
    }

    /// Builds a read-only [`CasSiteFs`] for a `cas <hash>` binding. Runs entirely
    /// outside the binding lock. Returns `None` when there is no backing store,
    /// the hash is malformed, or the manifest blob is missing/corrupt.
    fn resolve_cas(&self, hash: &str) -> Option<Arc<dyn FileSystem>> {
        let store = self.store.clone()?;
        let root = CasRootHash::from_hex(hash)?;
        let site = CasSiteFs::open_root(store, root).ok()?;
        Some(Arc::new(site))
    }

    fn descriptor_bytes(&self, host: &Host) -> FsResult<Vec<u8>> {
        let bindings = self.bindings.read().map_err(poisoned)?;
        bindings
            .get(host)
            .map(|source| source.descriptor().into_bytes())
            .ok_or(FsError::NotFound)
    }

    fn descriptor_len(&self, host: &Host) -> FsResult<u64> {
        Ok(self.descriptor_bytes(host)?.len() as u64)
    }

    /// Ensures a host exists with a placeholder source so a stat issued between
    /// open-for-write and the close-time commit succeeds (the 9P `Tlcreate`
    /// flow stats the path immediately after open), mirroring `#kv`.
    fn ensure_host(&self, host: &Host) -> FsResult<()> {
        let mut bindings = self.bindings.write().map_err(poisoned)?;
        bindings
            .entry(host.clone())
            .or_insert_with(|| SiteSource::Cas(String::new()));
        Ok(())
    }
}

fn poisoned(_: impl Sized) -> FsError {
    FsError::Other("sites device lock poisoned".to_owned())
}

enum SitesPath {
    Root,
    Host(Host),
}

fn parse_path(path: &NormalizedPath) -> FsResult<SitesPath> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(SitesPath::Root);
    }
    if raw.contains('/') {
        return Err(FsError::NotFound);
    }
    Host::registered(raw)
        .map(SitesPath::Host)
        .ok_or(FsError::NotFound)
}

impl FileSystem for SitesDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let SitesPath::Host(host) = parse_path(path)? else {
            return Err(FsError::IsDirectory);
        };
        if options.write || options.create || options.truncate {
            self.ensure_host(&host)?;
            return Ok(Box::new(SitesWriteFile::new(
                Arc::clone(&self.bindings),
                host,
            )));
        }
        if options.read {
            return Ok(Box::new(SitesReadFile::new(self.descriptor_bytes(&host)?)));
        }
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            SitesPath::Root => Ok(directory_metadata()),
            SitesPath::Host(host) => Ok(file_metadata(self.descriptor_len(&host)?)),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let SitesPath::Root = parse_path(path)? else {
            return Err(FsError::NotDirectory);
        };
        let bindings = self.bindings.read().map_err(poisoned)?;
        Ok(bindings
            .iter()
            .map(|(host, source)| {
                DirEntry::new(
                    host.as_str().to_owned(),
                    file_metadata(source.descriptor().len() as u64),
                )
            })
            .collect())
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let SitesPath::Host(host) = parse_path(path)? else {
            return Err(FsError::IsDirectory);
        };
        let mut bindings = self.bindings.write().map_err(poisoned)?;
        bindings.remove(&host).map(|_| ()).ok_or(FsError::NotFound)
    }
}

/// An empty read-only filesystem used as the placeholder a bare `memory`
/// descriptor binds to (a live source must be installed via `bind_site`).
#[derive(Debug)]
struct EmptySiteFs;

impl FileSystem for EmptySiteFs {
    fn open(&self, _path: &NormalizedPath, _options: OpenOptions) -> FsResult<Box<dyn File>> {
        Err(FsError::NotFound)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        if path.as_str() == "." {
            return Ok(directory_metadata());
        }
        Err(FsError::NotFound)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        if path.as_str() == "." {
            return Ok(Vec::new());
        }
        Err(FsError::NotDirectory)
    }
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

fn file_metadata(len: u64) -> Metadata {
    Metadata::new(FileType::File, len, modes::CONTROL_FILE)
}

#[cfg(test)]
mod tests;
