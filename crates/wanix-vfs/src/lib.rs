//! Plan 9-style namespace binding and resolution for Rust Wanix.
//!
//! This crate owns bind targets, bind order, direct and subpath resolution,
//! per-task namespace cloning, and synthesized directory views for unioned
//! bindings.

use std::collections::BTreeMap;
use std::fmt;

use wanix_fs::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};

mod binding;
mod mutation;
mod path;
mod readdir;
mod resolution;

#[cfg(test)]
mod tests;

use binding::BindTarget;
pub use binding::{BindOptions, BindPosition, Binding};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix namespace binding";

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
            return Ok(readdir::directory_metadata());
        }
        if self.has_synthetic_children(path) {
            return Ok(readdir::directory_metadata());
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
