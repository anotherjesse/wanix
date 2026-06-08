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
use std::sync::{Arc, RwLock};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

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
    /// An immutable CAS snapshot, named by its root hash (hex). Resolving this
    /// to a servable filesystem is Phase 3 (CAS-backed serving); until then it
    /// records the binding but does not serve.
    Cas(String),
}

impl SiteSource {
    /// Renders the stable, greppable control-file descriptor for this source,
    /// e.g. `memory\n` or `cas <root-hash>\n`.
    #[must_use]
    pub fn descriptor(&self) -> String {
        match self {
            Self::Memory(_) => "memory\n".to_owned(),
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
        let hash = trimmed
            .strip_prefix("cas")
            .map(str::trim)
            .filter(|hash| !hash.is_empty())?;
        Some(Self::Cas(hash.to_owned()))
    }
}

impl fmt::Debug for SiteSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Memory(..)"),
            Self::Cas(hash) => write!(f, "Cas({hash})"),
        }
    }
}

/// The `#sites` device: a host→source binding table exposed as a filesystem.
#[derive(Clone)]
pub struct SitesDevice {
    bindings: Bindings,
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
    /// Creates an empty site-binding device.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Arc::new(RwLock::new(BTreeMap::new())),
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

    /// Resolves a host to the filesystem to serve.
    ///
    /// The `Arc<dyn FileSystem>` is cloned out of the binding lock before
    /// returning, so the caller never holds the `#sites` lock while reading the
    /// served filesystem. A [`SiteSource::Cas`] resolves to `None` until Phase 3
    /// wires CAS-backed serving.
    #[must_use]
    pub fn resolve(&self, host: &Host) -> Option<Arc<dyn FileSystem>> {
        let source = {
            let bindings = self.bindings.read().ok()?;
            bindings.get(host).cloned()?
        };
        match source {
            SiteSource::Memory(fs) => Some(fs),
            SiteSource::Cas(_) => None,
        }
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
