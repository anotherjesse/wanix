use std::fs::{self, DirEntry as HostDirEntry};
use std::path::{Path, PathBuf};

use crate::{DirEntry, FileSystem, FsError, FsResult, NormalizedPath};

use super::{LocalFs, map_io_error};

impl LocalFs {
    pub(super) fn read_host_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let host_path = self.host_directory_path(path)?;
        let mut entries = self.read_host_dir_entries(path, &host_path)?;
        entries.sort_by(|left, right| left.name().cmp(right.name()));
        Ok(entries)
    }

    fn host_directory_path(&self, path: &NormalizedPath) -> FsResult<PathBuf> {
        let host_path = self.existing_host_path(path)?;
        let metadata = fs::metadata(&host_path).map_err(map_io_error)?;
        if metadata.is_dir() {
            Ok(host_path)
        } else {
            Err(FsError::NotDirectory)
        }
    }

    fn read_host_dir_entries(
        &self,
        path: &NormalizedPath,
        host_path: &Path,
    ) -> FsResult<Vec<DirEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(host_path).map_err(map_io_error)? {
            let entry = entry.map_err(map_io_error)?;
            if let Some(entry) = self.visible_host_dir_entry(path, entry)? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    fn visible_host_dir_entry(
        &self,
        parent_path: &NormalizedPath,
        entry: HostDirEntry,
    ) -> FsResult<Option<DirEntry>> {
        let name = host_entry_name(entry)?;
        if NormalizedPath::new(&name).is_err() {
            return Ok(None);
        }
        let child_path = child_entry_path(parent_path, &name)?;
        let metadata = match self.metadata(&child_path) {
            Ok(metadata) => metadata,
            Err(FsError::NotFound | FsError::PermissionDenied) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(DirEntry::new(name, metadata)))
    }
}

fn host_entry_name(entry: HostDirEntry) -> FsResult<String> {
    entry
        .file_name()
        .into_string()
        .map_err(|_| FsError::InvalidPath("<non-utf8 host path>".to_owned()))
}

fn child_entry_path(parent_path: &NormalizedPath, name: &str) -> FsResult<NormalizedPath> {
    if parent_path.as_str() == "." {
        NormalizedPath::new(name)
    } else {
        NormalizedPath::new(format!("{parent_path}/{name}"))
    }
}
