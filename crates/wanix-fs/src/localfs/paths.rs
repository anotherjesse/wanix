use std::fs;
use std::path::PathBuf;

use crate::{FsError, FsResult, MetadataLookup, NormalizedPath, OpenOptions};

use super::{LocalFs, map_io_error};

impl LocalFs {
    pub(super) fn existing_host_path(&self, path: &NormalizedPath) -> FsResult<PathBuf> {
        let host_path = self.raw_host_path(path);
        let resolved = fs::canonicalize(&host_path).map_err(map_io_error)?;
        if resolved.starts_with(&*self.root) {
            Ok(resolved)
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(super) fn host_path_for_open(
        &self,
        path: &NormalizedPath,
        options: OpenOptions,
    ) -> FsResult<PathBuf> {
        let host_path = self.raw_host_path(path);
        if options.create {
            match fs::symlink_metadata(&host_path) {
                Ok(_) => return self.existing_host_path(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = host_path.parent().ok_or(FsError::IsDirectory)?;
                    let parent = fs::canonicalize(parent).map_err(map_io_error)?;
                    if parent.starts_with(&*self.root) {
                        return Ok(host_path);
                    }
                    return Err(FsError::PermissionDenied);
                }
                Err(error) => return Err(map_io_error(error)),
            }
        }
        self.existing_host_path(path)
    }

    pub(super) fn raw_host_path(&self, path: &NormalizedPath) -> PathBuf {
        let mut host_path = (*self.root).clone();
        if path.as_str() != "." {
            for component in path.as_str().split('/') {
                host_path.push(component);
            }
        }
        host_path
    }

    pub(super) fn host_path_for_metadata(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<PathBuf> {
        if lookup.follow_symlinks() {
            return self.existing_host_path(path);
        }
        let host_path = self.raw_host_path(path);
        if path.as_str() == "." {
            return Ok(host_path);
        }
        let parent = host_path.parent().ok_or(FsError::PermissionDenied)?;
        let parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if parent.starts_with(&*self.root) {
            Ok(host_path)
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(super) fn host_path_for_final_component_operation(
        &self,
        path: &NormalizedPath,
    ) -> FsResult<PathBuf> {
        if path.as_str() == "." {
            return Err(FsError::AlreadyExists);
        }
        let host_path = self.raw_host_path(path);
        let parent = host_path.parent().ok_or(FsError::PermissionDenied)?;
        let parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if parent.starts_with(&*self.root) {
            Ok(host_path)
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(super) fn rename_source(&self, path: &NormalizedPath) -> FsResult<(PathBuf, fs::Metadata)> {
        let host_path = self.raw_host_path(path);
        let metadata = fs::symlink_metadata(&host_path).map_err(map_io_error)?;
        let resolved = fs::canonicalize(&host_path).map_err(map_io_error)?;
        if !resolved.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        Ok((host_path, metadata))
    }

    pub(super) fn rename_destination(
        &self,
        path: &NormalizedPath,
        old_metadata: &fs::Metadata,
    ) -> FsResult<PathBuf> {
        let host_path = self.raw_host_path(path);
        let parent = host_path.parent().ok_or(FsError::PermissionDenied)?;
        let parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if !parent.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        match fs::symlink_metadata(&host_path) {
            Ok(new_metadata) => validate_rename_replacement(
                self.root.as_ref().as_path(),
                old_metadata,
                &new_metadata,
                &host_path,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(map_io_error(error)),
        }?;
        Ok(host_path)
    }
}

fn validate_rename_replacement(
    root: &std::path::Path,
    old_metadata: &fs::Metadata,
    new_metadata: &fs::Metadata,
    new_host_path: &std::path::Path,
) -> FsResult<()> {
    let new_resolved = fs::canonicalize(new_host_path).map_err(map_io_error)?;
    if !new_resolved.starts_with(root) {
        return Err(FsError::PermissionDenied);
    }
    match (old_metadata.is_dir(), new_metadata.is_dir()) {
        (true, true) => ensure_directory_empty(new_host_path),
        (true, false) => Err(FsError::NotDirectory),
        (false, true) => Err(FsError::IsDirectory),
        (false, false) => Ok(()),
    }
}

fn ensure_directory_empty(path: &std::path::Path) -> FsResult<()> {
    if fs::read_dir(path)
        .map_err(map_io_error)?
        .next()
        .transpose()
        .map_err(map_io_error)?
        .is_some()
    {
        return Err(FsError::NotEmpty);
    }
    Ok(())
}
