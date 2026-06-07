//! Capsules: directory trees (worlds) frozen onto the content-addressed plane.
//!
//! A *capsule* is venti applied to a whole world: every file becomes a blob (so
//! identical files across worlds deduplicate to one blob), a deterministic
//! sorted [`WorldManifest`] mapping relative paths to file hashes is itself
//! serialized to a blob, and *that manifest blob's hash is the capsule id*. To
//! share a world you share its capsule id (in the mesh, a `BlobTicket`); to
//! receive it you fetch the manifest blob, then each referenced file blob, and
//! [`WorldManifest::materialize`] writes them out.
//!
//! Materialization is the trust boundary for an *incoming* capsule, so it is
//! defensive by construction:
//!
//! - **Path safety** — every manifest path is re-validated through
//!   [`wanix_fs::NormalizedPath`] (no `..`, no absolute, no escaping component)
//!   and rejected if it names a symlink-like or otherwise unsafe component,
//!   regardless of what the sender claimed. A hostile manifest cannot write
//!   outside the target directory.
//! - **Size + fan-out caps** — a whole-blob `get` loads a blob fully into
//!   memory, and a manifest with a huge fan-out forces many such loads, so the
//!   manifest is capped at [`MAX_MANIFEST_ENTRIES`] entries and
//!   [`CAPSULE_MANIFEST_MAX_BYTES`] serialized bytes, and every file blob is
//!   subject to the store's [`MAX_BLOB_SIZE`](crate::MAX_BLOB_SIZE) cap. One
//!   malicious ticket therefore cannot OOM or fill the disk.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use wanix_fs::{ContentHash, NormalizedPath};

use crate::store::{CasError, ContentStore};

/// Maximum number of entries a capsule manifest may carry.
///
/// Each entry forces a separate whole-blob fetch on materialize, so an
/// unbounded fan-out is a resource-exhaustion vector even when each blob is
/// individually small. 100k files covers a large source tree or rootfs.
pub const MAX_MANIFEST_ENTRIES: usize = 100_000;

/// Maximum serialized size, in bytes, of a capsule manifest blob.
///
/// The manifest blob is itself loaded whole into memory to be parsed, so its
/// size is bounded independently of the per-file blob cap. With ~104 bytes per
/// entry (path + 64-hex-char hash + separators) this comfortably admits
/// [`MAX_MANIFEST_ENTRIES`] entries while rejecting an absurd manifest.
pub const CAPSULE_MANIFEST_MAX_BYTES: usize = 16 * 1024 * 1024;

/// One file entry in a [`WorldManifest`]: a relative path and its blob hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// The file's path relative to the world root (validated, `/`-separated).
    pub path: String,
    /// The content hash of the file's bytes.
    pub hash: ContentHash,
}

/// A deterministic, sorted mapping of world-relative paths to file blob hashes.
///
/// Sorting by path makes the serialized form (and therefore the capsule id)
/// independent of insertion order, so the same world always freezes to the same
/// capsule id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldManifest {
    entries: Vec<ManifestEntry>,
}

/// The error surface of capsule freeze/materialize.
#[derive(Debug)]
pub enum MaterializeError {
    /// A manifest path was unsafe (absolute, contained `..`, or escaped root).
    UnsafePath(String),
    /// The manifest carried more than [`MAX_MANIFEST_ENTRIES`] entries.
    TooManyEntries(usize),
    /// The serialized manifest exceeded [`CAPSULE_MANIFEST_MAX_BYTES`].
    ManifestTooLarge(usize),
    /// The serialized manifest blob could not be parsed.
    MalformedManifest(String),
    /// A referenced blob was missing or failed verification in the store.
    Cas(CasError),
    /// A filesystem error while reading or writing the materialized tree.
    Io(String),
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafePath(path) => write!(f, "unsafe manifest path: {path}"),
            Self::TooManyEntries(count) => {
                write!(f, "manifest has {count} entries, exceeding the cap")
            }
            Self::ManifestTooLarge(len) => write!(f, "manifest blob of {len} bytes is too large"),
            Self::MalformedManifest(reason) => write!(f, "malformed manifest: {reason}"),
            Self::Cas(err) => write!(f, "{err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

impl From<CasError> for MaterializeError {
    fn from(err: CasError) -> Self {
        Self::Cas(err)
    }
}

/// Result type for capsule operations.
pub type MaterializeResult<T> = Result<T, MaterializeError>;

/// Counts produced by [`WorldManifest::materialize`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaterializeStats {
    /// Number of files written.
    pub files: usize,
    /// Total bytes written across all files.
    pub bytes: u64,
}

impl WorldManifest {
    /// Returns the sorted entries of this manifest.
    #[must_use]
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    /// Returns the number of entries in this manifest.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the manifest has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes the manifest to its deterministic blob form.
    ///
    /// One `\n`-terminated `"<hex-hash> <path>"` line per entry, entries sorted
    /// by path. This exact byte form is hashed to produce the capsule id, so it
    /// must stay stable.
    #[must_use]
    pub fn to_blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for entry in &self.entries {
            out.extend_from_slice(entry.hash.to_hex().as_bytes());
            out.push(b' ');
            out.extend_from_slice(entry.path.as_bytes());
            out.push(b'\n');
        }
        out
    }

    /// Parses a manifest from its serialized blob form, enforcing the caps.
    ///
    /// # Errors
    ///
    /// Returns [`MaterializeError::ManifestTooLarge`] /
    /// [`MaterializeError::TooManyEntries`] when the caps are exceeded,
    /// [`MaterializeError::UnsafePath`] when a path is not safe-relative, and
    /// [`MaterializeError::MalformedManifest`] on a structurally invalid line.
    pub fn from_blob(blob: &[u8]) -> MaterializeResult<Self> {
        if blob.len() > CAPSULE_MANIFEST_MAX_BYTES {
            return Err(MaterializeError::ManifestTooLarge(blob.len()));
        }
        let text = std::str::from_utf8(blob)
            .map_err(|_| MaterializeError::MalformedManifest("not utf-8".to_owned()))?;
        let mut entries = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            if entries.len() >= MAX_MANIFEST_ENTRIES {
                return Err(MaterializeError::TooManyEntries(entries.len() + 1));
            }
            let (hex, path) = line
                .split_once(' ')
                .ok_or_else(|| MaterializeError::MalformedManifest(line.to_owned()))?;
            let hash = ContentHash::from_hex(hex)
                .map_err(|_| MaterializeError::MalformedManifest(line.to_owned()))?;
            ensure_safe_relative(path)?;
            entries.push(ManifestEntry {
                path: path.to_owned(),
                hash,
            });
        }
        Ok(Self { entries })
    }

    /// Materializes every file in this manifest under `target`, fetching each
    /// blob from `store` and writing it path-safely.
    ///
    /// Every path is re-validated and confined to `target`, so a hostile
    /// manifest cannot escape it. Blob size is bounded by the store's own cap.
    ///
    /// # Errors
    ///
    /// Returns [`MaterializeError::UnsafePath`] for any path that escapes
    /// `target`, [`MaterializeError::Cas`] for a missing/oversized/corrupt blob,
    /// and [`MaterializeError::Io`] for a filesystem write failure.
    pub fn materialize(
        &self,
        store: &dyn ContentStore,
        target: &Path,
    ) -> MaterializeResult<MaterializeStats> {
        if self.entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(MaterializeError::TooManyEntries(self.entries.len()));
        }
        let mut stats = MaterializeStats::default();
        for entry in &self.entries {
            let relative = safe_join(target, &entry.path)?;
            let bytes = store.get(&entry.hash)?;
            if let Some(parent) = relative.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| MaterializeError::Io(err.to_string()))?;
            }
            std::fs::write(&relative, &bytes)
                .map_err(|err| MaterializeError::Io(err.to_string()))?;
            stats.files += 1;
            stats.bytes += bytes.len() as u64;
        }
        Ok(stats)
    }
}

/// A frozen world: its [`WorldManifest`] and the capsule id (manifest hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capsule {
    manifest: WorldManifest,
    id: ContentHash,
}

impl Capsule {
    /// Freezes the directory tree rooted at `root` into `store`, returning the
    /// capsule: each file is ingested as a blob, then the sorted manifest is
    /// ingested as a blob whose hash is the capsule id.
    ///
    /// # Errors
    ///
    /// Returns [`MaterializeError::TooManyEntries`] when the tree exceeds the
    /// fan-out cap, [`MaterializeError::Io`] on a read failure, and
    /// [`MaterializeError::Cas`] when a blob (file or manifest) cannot be stored.
    pub fn freeze(store: &dyn ContentStore, root: &Path) -> MaterializeResult<Self> {
        let mut entries = BTreeMap::new();
        collect_files(store, root, root, &mut entries)?;
        if entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(MaterializeError::TooManyEntries(entries.len()));
        }
        let manifest = WorldManifest {
            entries: entries
                .into_iter()
                .map(|(path, hash)| ManifestEntry { path, hash })
                .collect(),
        };
        let id = store.put(&manifest.to_blob())?;
        Ok(Self { manifest, id })
    }

    /// Loads a capsule from `store` by its capsule id (manifest blob hash).
    ///
    /// The manifest blob is fetched, size-capped, and parsed with full
    /// path-safety + fan-out validation before the capsule is returned, so a
    /// hostile id yields an error rather than an unsafe manifest.
    ///
    /// # Errors
    ///
    /// Returns [`MaterializeError::Cas`] when the manifest blob is missing or
    /// corrupt and the manifest-parse errors when the caps or path-safety checks
    /// fail.
    pub fn load(store: &dyn ContentStore, id: ContentHash) -> MaterializeResult<Self> {
        let blob = store.get(&id)?;
        let manifest = WorldManifest::from_blob(&blob)?;
        Ok(Self { manifest, id })
    }

    /// Returns the capsule id: the content hash of the serialized manifest.
    #[must_use]
    pub fn id(&self) -> ContentHash {
        self.id
    }

    /// Returns the capsule's manifest.
    #[must_use]
    pub fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    /// Materializes the capsule's world under `target` from `store`.
    ///
    /// # Errors
    ///
    /// Propagates [`WorldManifest::materialize`] errors.
    pub fn materialize(
        &self,
        store: &dyn ContentStore,
        target: &Path,
    ) -> MaterializeResult<MaterializeStats> {
        self.manifest.materialize(store, target)
    }
}

/// Recursively ingests files under `dir` into `store`, keyed by `root`-relative
/// path. Symlinks are skipped (a capsule carries content, not link topology).
fn collect_files(
    store: &dyn ContentStore,
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, ContentHash>,
) -> MaterializeResult<()> {
    let read = std::fs::read_dir(dir).map_err(|err| MaterializeError::Io(err.to_string()))?;
    for entry in read {
        let entry = entry.map_err(|err| MaterializeError::Io(err.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| MaterializeError::Io(err.to_string()))?;
        let path = entry.path();
        if file_type.is_symlink() {
            // A capsule freezes content; link topology is not represented, so a
            // symlink is skipped rather than dereferenced (which could escape).
            continue;
        }
        if file_type.is_dir() {
            collect_files(store, root, &path, out)?;
        } else if file_type.is_file() {
            let rel = relative_key(root, &path)?;
            let bytes =
                std::fs::read(&path).map_err(|err| MaterializeError::Io(err.to_string()))?;
            let hash = store.put(&bytes)?;
            out.insert(rel, hash);
        }
    }
    Ok(())
}

/// Computes the `/`-separated, safe-relative key of `path` under `root`.
fn relative_key(root: &Path, path: &Path) -> MaterializeResult<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| MaterializeError::UnsafePath(path.display().to_string()))?;
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| MaterializeError::UnsafePath(path.display().to_string()))?
                    .to_owned(),
            ),
            _ => return Err(MaterializeError::UnsafePath(path.display().to_string())),
        }
    }
    let key = parts.join("/");
    ensure_safe_relative(&key)?;
    Ok(key)
}

/// Rejects any path that is not a safe relative Wanix path (no `..`, absolute,
/// or empty component), reusing [`NormalizedPath`]'s validation.
fn ensure_safe_relative(path: &str) -> MaterializeResult<()> {
    NormalizedPath::new(path).map_err(|_| MaterializeError::UnsafePath(path.to_owned()))?;
    Ok(())
}

/// Joins a validated relative `path` onto `target`, confining the result inside
/// `target` even against `..`-laden input.
fn safe_join(target: &Path, path: &str) -> MaterializeResult<PathBuf> {
    ensure_safe_relative(path)?;
    let mut out = target.to_path_buf();
    for part in path.split('/') {
        // `ensure_safe_relative` already rejected `..`/`.`/empty, but we double
        // check here so this function is safe to call on any input.
        if part.is_empty() || part == "." || part == ".." {
            return Err(MaterializeError::UnsafePath(path.to_owned()));
        }
        out.push(part);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
