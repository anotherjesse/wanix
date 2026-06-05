use std::sync::Arc;

use wanix_fs::{FsError, FsResult, NormalizedPath};

use super::Namespace;
use crate::path::ResolvedTarget;

impl Namespace {
    pub(super) fn try_resolved_path<T>(
        &self,
        path: &NormalizedPath,
        mut operation: impl FnMut(&ResolvedTarget) -> FsResult<T>,
        synthetic_error: impl FnOnce(&NormalizedPath) -> FsError,
    ) -> FsResult<T> {
        let mut saw_not_directory = false;
        for target in self.resolve_candidates(path)? {
            match operation(&target) {
                Ok(value) => return Ok(value),
                Err(FsError::NotDirectory) => saw_not_directory = true,
                Err(FsError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }

        if self.has_synthetic_children(path) {
            return Err(synthetic_error(path));
        }
        if saw_not_directory {
            return Err(FsError::NotDirectory);
        }
        Err(FsError::NotFound)
    }

    pub(super) fn same_backing_mutation_targets(
        &self,
        old_path: &NormalizedPath,
        new_path: &NormalizedPath,
    ) -> FsResult<(ResolvedTarget, ResolvedTarget)> {
        if old_path.as_str() == "." || new_path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }

        let new_candidates = self.destination_candidates(new_path)?;
        self.source_and_same_filesystem_destination(old_path, new_candidates)
    }

    fn destination_candidates(&self, new_path: &NormalizedPath) -> FsResult<Vec<ResolvedTarget>> {
        let new_candidates = self.resolve_candidates(new_path)?;
        if !new_candidates.is_empty() {
            return Ok(new_candidates);
        }
        if self.has_synthetic_children(new_path) {
            return Err(FsError::AlreadyExists);
        }
        Err(FsError::NotFound)
    }

    fn source_and_same_filesystem_destination(
        &self,
        old_path: &NormalizedPath,
        new_candidates: Vec<ResolvedTarget>,
    ) -> FsResult<(ResolvedTarget, ResolvedTarget)> {
        let mut saw_not_directory = false;
        for old_target in self.resolve_candidates(old_path)? {
            match old_target.filesystem.metadata(&old_target.path) {
                Ok(_) => return same_filesystem_pair(old_target, &new_candidates),
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
}

fn same_filesystem_pair(
    old_target: ResolvedTarget,
    new_candidates: &[ResolvedTarget],
) -> FsResult<(ResolvedTarget, ResolvedTarget)> {
    for new_target in new_candidates {
        if Arc::ptr_eq(&old_target.filesystem, &new_target.filesystem) {
            return Ok((old_target, new_target.clone()));
        }
    }
    Err(FsError::NotSupported)
}
