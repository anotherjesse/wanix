//! Plan 9-style namespace binding and resolution for Rust Wanix.
//!
//! This crate owns bind targets, bind order, direct and subpath resolution,
//! per-task namespace cloning, and synthesized directory views for unioned
//! bindings.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, MetadataLookup,
    NormalizedPath, OpenOptions,
};

#[cfg(test)]
mod tests;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix namespace binding";

/// Position used when inserting a binding at a destination path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BindPosition {
    /// Place the binding before existing bindings at the destination.
    #[default]
    First,
    /// Replace existing bindings at the destination.
    Replace,
    /// Place the binding after existing bindings at the destination.
    Last,
}

/// Options for namespace binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindOptions {
    /// Where to insert the binding at the destination.
    pub position: BindPosition,
}

/// A public view of a stored binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    source: NormalizedPath,
    destination: NormalizedPath,
}

impl Binding {
    /// Returns the source path.
    #[must_use]
    pub fn source(&self) -> &NormalizedPath {
        &self.source
    }

    /// Returns the destination path.
    #[must_use]
    pub fn destination(&self) -> &NormalizedPath {
        &self.destination
    }
}

#[derive(Clone)]
struct BindTarget {
    filesystem: Arc<dyn FileSystem>,
    source: NormalizedPath,
}

/// A Wanix namespace containing bind targets.
#[derive(Clone, Default)]
pub struct Namespace {
    bindings: BTreeMap<NormalizedPath, Vec<BindTarget>>,
}

impl fmt::Debug for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Namespace")
            .field("binding_count", &self.binding_count())
            .finish()
    }
}

impl Namespace {
    /// Creates an empty namespace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a filesystem binding.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if either path is invalid, the source path
    /// does not exist, or the destination path is invalid.
    pub fn bind(
        &mut self,
        filesystem: Arc<dyn FileSystem>,
        source: impl AsRef<str>,
        destination: impl AsRef<str>,
        options: BindOptions,
    ) -> FsResult<()> {
        let source = NormalizedPath::new(source)?;
        let destination = NormalizedPath::new(destination)?;
        filesystem.metadata(&source)?;

        let target = BindTarget { filesystem, source };
        let targets = self.bindings.entry(destination).or_default();
        match options.position {
            BindPosition::First => targets.insert(0, target),
            BindPosition::Replace => {
                targets.clear();
                targets.push(target);
            }
            BindPosition::Last => targets.push(target),
        }
        Ok(())
    }

    /// Returns stored bindings in destination order, then resolution order.
    #[must_use]
    pub fn bindings(&self) -> Vec<Binding> {
        self.bindings
            .iter()
            .flat_map(|(destination, targets)| {
                targets.iter().map(|target| Binding {
                    source: target.source.clone(),
                    destination: destination.clone(),
                })
            })
            .collect()
    }

    /// Returns the number of bind targets stored in this namespace.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.values().map(Vec::len).sum()
    }

    fn resolve_candidates(&self, path: &NormalizedPath) -> Vec<ResolvedTarget> {
        let mut candidates = Vec::new();
        for (destination, targets) in &self.bindings {
            let Some(relative) = relative_to_destination(path, destination) else {
                continue;
            };
            for target in targets {
                candidates.push(ResolvedTarget {
                    filesystem: Arc::clone(&target.filesystem),
                    path: join_paths(&target.source, relative),
                    destination_len: destination.as_str().len(),
                });
            }
        }
        candidates.sort_by_key(|candidate| Reverse(candidate.destination_len));
        candidates
    }

    fn has_synthetic_children(&self, path: &NormalizedPath) -> bool {
        self.bindings
            .keys()
            .any(|destination| immediate_child_name(destination, path).is_some())
    }

    fn synthetic_child_metadata(targets: &[BindTarget]) -> Option<Metadata> {
        Self::synthetic_child_metadata_with_lookup(targets, MetadataLookup::FollowSymlink)
    }

    fn synthetic_child_metadata_with_lookup(
        targets: &[BindTarget],
        lookup: MetadataLookup,
    ) -> Option<Metadata> {
        targets.iter().find_map(|target| {
            target
                .filesystem
                .metadata_with_lookup(&target.source, lookup)
                .ok()
        })
    }
}

impl FileSystem for Namespace {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let mut saw_directory = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.open(&target.path, options) {
                Ok(file) => return Ok(file),
                Err(FsError::IsDirectory) => saw_directory = true,
                Err(FsError::NotFound | FsError::NotDirectory) => {}
                Err(err) => return Err(err),
            }
        }
        if saw_directory || self.has_synthetic_children(path) {
            return Err(FsError::IsDirectory);
        }
        Err(FsError::NotFound)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.metadata_with_lookup(path, MetadataLookup::FollowSymlink)
    }

    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        for target in self.resolve_candidates(path) {
            match target.filesystem.metadata_with_lookup(&target.path, lookup) {
                Ok(metadata) => return Ok(metadata),
                Err(FsError::NotFound | FsError::NotDirectory) => {}
                Err(err) => return Err(err),
            }
        }
        if path.as_str() == "." {
            return Ok(directory_metadata());
        }
        if self.has_synthetic_children(path) {
            return Ok(directory_metadata());
        }
        Err(FsError::NotFound)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let mut entries = BTreeMap::<String, Metadata>::new();
        let mut found_directory = path.as_str() == ".";

        for target in self.resolve_candidates(path) {
            match target.filesystem.metadata(&target.path) {
                Ok(metadata) if metadata.file_type() != FileType::Directory => {
                    return Err(FsError::NotDirectory);
                }
                Ok(_) => {}
                Err(FsError::NotFound | FsError::NotDirectory) => continue,
                Err(err) => return Err(err),
            }
            match target.filesystem.read_dir(&target.path) {
                Ok(target_entries) => {
                    found_directory = true;
                    for entry in target_entries {
                        if !is_hidden(entry.name()) {
                            entries
                                .entry(entry.name().to_owned())
                                .or_insert_with(|| entry.metadata().clone());
                        }
                    }
                }
                Err(FsError::NotFound | FsError::NotDirectory) => {}
                Err(err) => return Err(err),
            }
        }

        for (destination, targets) in &self.bindings {
            if let Some(child) = immediate_child_name(destination, path) {
                found_directory = true;
                if is_hidden(child) {
                    continue;
                }
                if is_direct_child(destination, path) {
                    if let Some(metadata) = Self::synthetic_child_metadata(targets) {
                        entries.insert(child.to_owned(), metadata);
                    }
                } else {
                    entries
                        .entry(child.to_owned())
                        .or_insert_with(directory_metadata);
                }
            }
        }

        if !found_directory {
            return Err(FsError::NotFound);
        }

        Ok(entries
            .into_iter()
            .map(|(name, metadata)| DirEntry::new(name, metadata))
            .collect())
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.read_link(&target.path) {
                Ok(target) => return Ok(target),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if self.has_synthetic_children(path) {
            return Err(FsError::InvalidPath(format!("{path} is not a symlink")));
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        let mut saw_not_directory = false;
        for bind_target in self.resolve_candidates(path) {
            match bind_target.filesystem.symlink(target, &bind_target.path) {
                Ok(()) => return Ok(()),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if self.has_synthetic_children(path) {
            return Err(FsError::AlreadyExists);
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.create_dir(&target.path) {
                Ok(()) => return Ok(()),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        if self.has_synthetic_children(path) {
            return Err(FsError::AlreadyExists);
        }
        Err(FsError::NotFound)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let mut saw_directory = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.remove_file(&target.path) {
                Ok(()) => return Ok(()),
                Err(FsError::IsDirectory) => saw_directory = true,
                Err(FsError::NotFound | FsError::NotDirectory) => {}
                Err(err) => return Err(err),
            }
        }
        if saw_directory || self.has_synthetic_children(path) {
            return Err(FsError::IsDirectory);
        }
        Err(FsError::NotFound)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }
        let mut saw_not_directory = false;
        let mut saw_not_empty = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.remove_dir(&target.path) {
                Ok(()) => return Ok(()),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotEmpty) => saw_not_empty = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if saw_not_empty || self.has_synthetic_children(path) {
            return Err(FsError::NotEmpty);
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        if old_path.as_str() == "." || new_path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }

        let new_candidates = self.resolve_candidates(new_path);
        if new_candidates.is_empty() {
            if self.has_synthetic_children(new_path) {
                return Err(FsError::AlreadyExists);
            }
            return Err(FsError::NotFound);
        }

        let mut saw_not_directory = false;
        for old_target in self.resolve_candidates(old_path) {
            match old_target.filesystem.metadata(&old_target.path) {
                Ok(_) => {
                    for new_target in &new_candidates {
                        if Arc::ptr_eq(&old_target.filesystem, &new_target.filesystem) {
                            return old_target
                                .filesystem
                                .rename(&old_target.path, &new_target.path);
                        }
                    }
                    return Err(FsError::NotSupported);
                }
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }

        if self.has_synthetic_children(old_path) {
            return Err(FsError::NotSupported);
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path) {
            match target.filesystem.set_permissions(&target.path, permissions) {
                Ok(()) => return Ok(()),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if self.has_synthetic_children(path) {
            return Err(FsError::NotSupported);
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path) {
            match target
                .filesystem
                .set_times(&target.path, accessed_time_ns, modified_time_ns)
            {
                Ok(()) => return Ok(()),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        if self.has_synthetic_children(path) {
            return Err(FsError::NotSupported);
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }
}

struct ResolvedTarget {
    filesystem: Arc<dyn FileSystem>,
    path: NormalizedPath,
    destination_len: usize,
}

fn relative_to_destination<'a>(
    path: &'a NormalizedPath,
    destination: &NormalizedPath,
) -> Option<&'a str> {
    if destination.as_str() == "." {
        return Some(if path.as_str() == "." {
            ""
        } else {
            path.as_str()
        });
    }
    if path == destination {
        return Some("");
    }
    path.as_str()
        .strip_prefix(destination.as_str())?
        .strip_prefix('/')
}

fn join_paths(base: &NormalizedPath, relative: &str) -> NormalizedPath {
    if relative.is_empty() {
        return base.clone();
    }
    if base.as_str() == "." {
        NormalizedPath::new(relative).expect("relative path is already normalized")
    } else {
        NormalizedPath::new(format!("{base}/{relative}"))
            .expect("joined bind path is already normalized")
    }
}

fn immediate_child_name<'a>(
    destination: &'a NormalizedPath,
    parent: &NormalizedPath,
) -> Option<&'a str> {
    if destination == parent {
        return None;
    }

    let rest = if parent.as_str() == "." {
        destination.as_str()
    } else {
        destination
            .as_str()
            .strip_prefix(parent.as_str())?
            .strip_prefix('/')?
    };

    if rest.is_empty() {
        return None;
    }
    Some(
        rest.split('/')
            .next()
            .expect("split always has one segment"),
    )
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('#')
}

fn is_direct_child(destination: &NormalizedPath, parent: &NormalizedPath) -> bool {
    destination.parent().as_ref() == Some(parent)
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o755)
}
