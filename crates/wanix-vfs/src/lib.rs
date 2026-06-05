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

mod mutation;
mod path;
mod readdir;

#[cfg(test)]
mod tests;

use path::{
    ResolvedTarget, immediate_child_name, is_direct_child, join_paths, relative_to_destination,
};

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

    fn resolve_candidates(&self, path: &NormalizedPath) -> FsResult<Vec<ResolvedTarget>> {
        let mut candidates = Vec::new();
        for (destination, targets) in &self.bindings {
            let Some(relative) = relative_to_destination(path, destination) else {
                continue;
            };
            for target in targets {
                candidates.push(ResolvedTarget {
                    filesystem: Arc::clone(&target.filesystem),
                    path: join_paths(&target.source, relative)?,
                    destination_len: destination.as_str().len(),
                });
            }
        }
        candidates.sort_by_key(|candidate| Reverse(candidate.destination_len));
        Ok(candidates)
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
        for target in self.resolve_candidates(path)? {
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
        for target in self.resolve_candidates(path)? {
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
        self.read_namespace_dir(path)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        self.try_resolved_path(
            path,
            |target| target.filesystem.read_link(&target.path),
            |path| FsError::InvalidPath(format!("{path} is not a symlink")),
        )
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        self.try_resolved_path(
            path,
            |bind_target| bind_target.filesystem.symlink(target, &bind_target.path),
            |_| FsError::AlreadyExists,
        )
    }

    fn hard_link(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        let (old_target, new_target) = self.same_backing_mutation_targets(old_path, new_path)?;
        old_target
            .filesystem
            .hard_link(&old_target.path, &new_target.path)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path)? {
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
        for target in self.resolve_candidates(path)? {
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
        for target in self.resolve_candidates(path)? {
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
        let (old_target, new_target) = self.same_backing_mutation_targets(old_path, new_path)?;
        old_target
            .filesystem
            .rename(&old_target.path, &new_target.path)
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path)? {
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
        for target in self.resolve_candidates(path)? {
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

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o755)
}
