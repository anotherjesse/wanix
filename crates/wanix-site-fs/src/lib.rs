//! FileSystem-backed static site resolution.
//!
//! Wanix's north star is "a site is a `FileSystem`": if the HTTP server reads
//! through the [`FileSystem`] trait instead of the host disk, then any
//! filesystem (an in-memory generator output, a host directory, a CAS snapshot,
//! a mesh mount) becomes servable with one code path. This crate is that one
//! code path: [`read_site_file`] resolves a request URL against a filesystem,
//! applying directory-index resolution and confining the request to the
//! filesystem's own root — with no `std::fs` and no host canonicalization.

#[cfg(test)]
mod tests;

use wanix_fs::{File, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

/// Maximum bytes pulled per [`File::read`] call while draining a site file.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// A request path already resolved to a safe in-filesystem path, with
/// directory-index resolution applied.
///
/// Holds a [`NormalizedPath`], so it can never carry `..`, an absolute prefix,
/// or an empty component — the path rules of `wanix-fs` enforce the confinement
/// that the previous `std::fs::canonicalize` + `starts_with` guard provided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteRequestPath(NormalizedPath);

impl SiteRequestPath {
    /// Returns the resolved in-filesystem path.
    #[must_use]
    pub fn as_normalized(&self) -> &NormalizedPath {
        &self.0
    }
}

/// Outcome of resolving a URL path against a site filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteFile {
    /// A regular file: its bytes and the lowercased trailing extension (if any)
    /// used by the caller's content-type table.
    Found {
        /// The file's bytes.
        bytes: Vec<u8>,
        /// The lowercased trailing extension, e.g. `html`, or `None`.
        extension: Option<String>,
    },
    /// No file matched (or a directory had no `index.html`).
    NotFound,
    /// The path was malformed or escaped the filesystem root.
    Forbidden,
}

/// Resolves `url_relative` against `fs`, applying directory-index resolution,
/// and returns the file bytes.
///
/// `url_relative` is the percent-decoded request path with no leading `/`
/// (possibly empty, meaning `/`). Resolution mirrors the prior `std::fs` static
/// handler: a directory retargets to its `index.html`, and `..`/absolute paths
/// are rejected at [`NormalizedPath`] construction (so there is no host
/// canonicalize and no symlink-escape window to reason about — `MemFs`/`LocalFs`
/// confine within their own root). Pure filesystem access; no host disk.
pub fn read_site_file(fs: &dyn FileSystem, url_relative: &str) -> SiteFile {
    match resolve_site_path(fs, url_relative) {
        Ok(Some(path)) => read_resolved_file(fs, &path),
        Ok(None) => SiteFile::NotFound,
        Err(site_file) => site_file,
    }
}

/// Resolves the URL to a concrete file path, applying directory-index lookup.
///
/// `Ok(Some(path))` is a file to read; `Ok(None)` is a clean miss; `Err(..)`
/// carries an early [`SiteFile::NotFound`]/[`SiteFile::Forbidden`] outcome.
fn resolve_site_path(
    fs: &dyn FileSystem,
    url_relative: &str,
) -> Result<Option<SiteRequestPath>, SiteFile> {
    let trimmed = url_relative.trim_matches('/');
    let raw = if trimmed.is_empty() { "." } else { trimmed };
    let path = NormalizedPath::new(raw).map_err(|_| SiteFile::Forbidden)?;

    match fs.metadata(&path) {
        Ok(metadata) if metadata.file_type() == FileType::Directory => {
            let index = directory_index_path(&path).ok_or(SiteFile::Forbidden)?;
            Ok(Some(index))
        }
        Ok(_) => Ok(Some(SiteRequestPath(path))),
        Err(FsError::NotFound) => Ok(None),
        Err(FsError::PermissionDenied) => Err(SiteFile::Forbidden),
        Err(_) => Ok(None),
    }
}

/// Builds the `<dir>/index.html` path for a directory request.
fn directory_index_path(dir: &NormalizedPath) -> Option<SiteRequestPath> {
    let joined = match dir.as_str() {
        "." => "index.html".to_owned(),
        other => format!("{other}/index.html"),
    };
    NormalizedPath::new(joined).ok().map(SiteRequestPath)
}

/// Opens and drains the resolved file, mapping filesystem errors to outcomes.
fn read_resolved_file(fs: &dyn FileSystem, path: &SiteRequestPath) -> SiteFile {
    let mut file = match fs.open(path.as_normalized(), OpenOptions::read()) {
        Ok(file) => file,
        Err(FsError::NotFound) => return SiteFile::NotFound,
        Err(FsError::PermissionDenied) => return SiteFile::Forbidden,
        Err(_) => return SiteFile::NotFound,
    };
    match drain_file(file.as_mut()) {
        Some(bytes) => SiteFile::Found {
            bytes,
            extension: split_extension(path.as_normalized()),
        },
        None => SiteFile::NotFound,
    }
}

/// Reads a file to end in [`READ_CHUNK_BYTES`] chunks; `None` on read error.
fn drain_file(file: &mut dyn File) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return Some(bytes),
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(_) => return None,
        }
    }
}

/// Returns the lowercased trailing extension of `path`, if any, so the caller
/// can drive its existing content-type table.
fn split_extension(path: &NormalizedPath) -> Option<String> {
    let name = path.file_name();
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}
