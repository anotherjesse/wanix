//! Directory-tree synthesis over a flat [`wanix_cas::WorldManifest`].
//!
//! A manifest is a sorted list of `path -> hash` *file* entries; it has no
//! explicit directory records. The served filesystem tree is implied by the
//! path prefixes, so this module synthesizes directory metadata and `read_dir`
//! results from those prefixes without storing a tree.

use std::collections::BTreeMap;

use wanix_fs::{DirEntry, FileType, Metadata};

use crate::Entry;

/// Unix mode bits reported for files served from a frozen site (read-only).
pub(crate) const FILE_MODE: u32 = 0o444;
/// Unix mode bits reported for synthesized directories (read-only, traversable).
pub(crate) const DIR_MODE: u32 = 0o555;

/// What a path resolves to within a frozen site's manifest.
pub(crate) enum Resolved<'a> {
    /// A file entry: serve its blob by hash.
    File(&'a Entry),
    /// A directory implied by one or more entries beneath it.
    Directory,
    /// No entry matches the path or any prefix.
    Missing,
}

/// Resolves `rel` (a `/`-separated, root-relative path, or `""`/`"."` for the
/// site root) against the manifest's file entries.
pub(crate) fn resolve<'a>(entries: &'a [Entry], rel: &str) -> Resolved<'a> {
    let rel = normalize(rel);
    if rel.is_empty() {
        return Resolved::Directory;
    }
    if let Some(entry) = entries.iter().find(|entry| entry.path == rel) {
        return Resolved::File(entry);
    }
    // A path that is a strict directory prefix of some entry is a directory.
    let dir_prefix = format!("{rel}/");
    if entries
        .iter()
        .any(|entry| entry.path.starts_with(&dir_prefix))
    {
        return Resolved::Directory;
    }
    Resolved::Missing
}

/// Lists the immediate children of directory `rel` (`""`/`"."` for the root),
/// synthesizing one [`DirEntry`] per distinct next path segment.
pub(crate) fn read_dir(entries: &[Entry], rel: &str) -> Vec<DirEntry> {
    let rel = normalize(rel);
    let prefix = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    };
    // Map each immediate child name to whether it is itself a file or a directory
    // (a directory has further `/`-separated segments under it).
    let mut children: BTreeMap<String, ChildKind> = BTreeMap::new();
    for entry in entries {
        let Some(rest) = entry.path.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((segment, _)) => {
                children.insert(segment.to_owned(), ChildKind::Directory);
            }
            None => {
                children.entry(rest.to_owned()).or_insert(ChildKind::File);
            }
        }
    }
    children
        .into_iter()
        .map(|(name, kind)| DirEntry::new(name, kind.metadata()))
        .collect()
}

/// Whether a synthesized child is a leaf file or an implied directory.
///
/// `read_dir` reports a zero length for file children rather than fetching every
/// blob to learn its size — directory listing must stay cheap, and the Phase 0
/// static handler stats a concrete path (not a directory entry) before serving.
enum ChildKind {
    File,
    Directory,
}

impl ChildKind {
    fn metadata(&self) -> Metadata {
        match self {
            Self::File => file_metadata(0),
            Self::Directory => directory_metadata(),
        }
    }
}

/// Builds file metadata for a leaf of the given byte length.
pub(crate) fn file_metadata(len: u64) -> Metadata {
    Metadata::new(FileType::File, len, FILE_MODE)
}

/// Builds metadata for a synthesized directory.
pub(crate) fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 0, DIR_MODE)
}

/// Normalizes a request-relative path: `"."`/`""` become the empty root key,
/// and any leading/trailing slashes are trimmed.
fn normalize(rel: &str) -> &str {
    let trimmed = rel.trim_matches('/');
    if trimmed == "." { "" } else { trimmed }
}
