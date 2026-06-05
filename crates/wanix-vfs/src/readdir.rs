use std::collections::BTreeMap;

use wanix_fs::{DirEntry, FileType, FsError, FsResult, Metadata, NormalizedPath};

use super::{BindTarget, Namespace};
use crate::path::{ResolvedTarget, immediate_child_name, is_direct_child};

impl Namespace {
    pub(super) fn read_namespace_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let mut view = DirectoryView::new(path);
        view.add_resolved_entries(self.resolve_candidates(path)?)?;
        view.add_synthetic_entries(&self.bindings, path);
        view.into_entries()
    }
}

struct DirectoryView {
    entries: BTreeMap<String, Metadata>,
    found_directory: bool,
}

impl DirectoryView {
    fn new(path: &NormalizedPath) -> Self {
        Self {
            entries: BTreeMap::new(),
            found_directory: path.as_str() == ".",
        }
    }

    fn add_resolved_entries(&mut self, targets: Vec<ResolvedTarget>) -> FsResult<()> {
        for target in targets {
            self.add_target_entries(target)?;
        }
        Ok(())
    }

    fn add_target_entries(&mut self, target: ResolvedTarget) -> FsResult<()> {
        match target.filesystem.metadata(&target.path) {
            Ok(metadata) if metadata.file_type() != FileType::Directory => {
                return Err(FsError::NotDirectory);
            }
            Ok(_) => {}
            Err(FsError::NotFound | FsError::NotDirectory) => return Ok(()),
            Err(err) => return Err(err),
        }
        match target.filesystem.read_dir(&target.path) {
            Ok(target_entries) => {
                self.found_directory = true;
                for entry in target_entries {
                    if !is_hidden(entry.name()) {
                        self.entries
                            .entry(entry.name().to_owned())
                            .or_insert_with(|| entry.metadata().clone());
                    }
                }
            }
            Err(FsError::NotFound | FsError::NotDirectory) => {}
            Err(err) => return Err(err),
        }
        Ok(())
    }

    fn add_synthetic_entries(
        &mut self,
        bindings: &BTreeMap<NormalizedPath, Vec<BindTarget>>,
        parent: &NormalizedPath,
    ) {
        for (destination, targets) in bindings {
            let Some(child) = immediate_child_name(destination, parent) else {
                continue;
            };
            self.found_directory = true;
            if is_hidden(child) {
                continue;
            }
            if is_direct_child(destination, parent) {
                if let Some(metadata) = Namespace::synthetic_child_metadata(targets) {
                    self.entries.insert(child.to_owned(), metadata);
                }
            } else {
                self.entries
                    .entry(child.to_owned())
                    .or_insert_with(directory_metadata);
            }
        }
    }

    fn into_entries(self) -> FsResult<Vec<DirEntry>> {
        if !self.found_directory {
            return Err(FsError::NotFound);
        }

        Ok(self
            .entries
            .into_iter()
            .map(|(name, metadata)| DirEntry::new(name, metadata))
            .collect())
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('#')
}

pub(super) fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o755)
}
