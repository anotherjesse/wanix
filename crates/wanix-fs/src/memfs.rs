use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::{DirEntry, File, FileSystem, FsError, FsResult, Metadata, NormalizedPath, OpenOptions};

mod file;
mod node;
mod open;
mod rename;
mod tree;

use file::MemFile;
use node::{DEFAULT_DIR_MODE, DEFAULT_FILE_MODE, Node, PERMISSION_MODE_MASK};
use open::prepare_open;
use rename::rename_node_tree;
use tree::direct_children;

/// In-memory filesystem used by the first Rust Wanix tests and demos.
#[derive(Debug, Clone)]
pub struct MemFs {
    nodes: Arc<RwLock<BTreeMap<NormalizedPath, Node>>>,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// Creates an empty filesystem with an explicit root directory.
    #[must_use]
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(NormalizedPath::root(), Node::dir(DEFAULT_DIR_MODE));
        Self {
            nodes: Arc::new(RwLock::new(nodes)),
        }
    }

    /// Creates all directories needed for `path`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when a path component exists as a file.
    pub fn create_dir_all(&self, path: impl AsRef<str>) -> FsResult<()> {
        let path = NormalizedPath::new(path)?;
        let mut stack = Vec::new();
        let mut cursor = Some(path);
        while let Some(current) = cursor {
            stack.push(current.clone());
            cursor = current.parent();
        }

        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        for dir in stack.into_iter().rev() {
            match nodes.get(&dir) {
                Some(node) if !node.is_directory() => {
                    return Err(FsError::NotDirectory);
                }
                Some(_) => {}
                None => {
                    nodes.insert(dir, Node::dir(DEFAULT_DIR_MODE));
                }
            }
        }
        Ok(())
    }

    /// Writes a complete file, creating implicit parent directories.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the path is invalid or names a directory.
    pub fn write_file(&self, path: impl AsRef<str>, data: impl AsRef<[u8]>) -> FsResult<()> {
        let path = NormalizedPath::new(path)?;
        if path.as_str() == "." {
            return Err(FsError::IsDirectory);
        }
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent.as_str())?;
        }

        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        if matches!(nodes.get(&path), Some(node) if node.is_directory()) {
            return Err(FsError::IsDirectory);
        }
        nodes.insert(path, Node::file(data.as_ref().to_vec(), DEFAULT_FILE_MODE));
        Ok(())
    }

    /// Reads a complete file into memory.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file is missing or is a directory.
    pub fn read_file(&self, path: impl AsRef<str>) -> FsResult<Vec<u8>> {
        let path = NormalizedPath::new(path)?;
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get(&path).ok_or(FsError::NotFound)?;
        if node.is_directory() {
            return Err(FsError::IsDirectory);
        }
        Ok(node.data.clone())
    }

    fn metadata_for(
        nodes: &BTreeMap<NormalizedPath, Node>,
        path: &NormalizedPath,
    ) -> FsResult<Metadata> {
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        let len = if node.is_directory() {
            2 + direct_children(nodes, path).count() as u64
        } else {
            node.data.len() as u64
        };
        Ok(node.metadata(len))
    }
}

impl FileSystem for MemFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        {
            let mut nodes = self
                .nodes
                .write()
                .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
            prepare_open(&mut nodes, path, options)?;
        }

        Ok(Box::new(MemFile::new(
            Arc::clone(&self.nodes),
            path.clone(),
            options,
        )))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        Self::metadata_for(&nodes, path)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if !node.is_directory() {
            return Err(FsError::NotDirectory);
        }

        direct_children(&nodes, path)
            .map(|child| {
                let metadata = Self::metadata_for(&nodes, child)?;
                Ok(DirEntry::new(child.file_name(), metadata))
            })
            .collect()
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::AlreadyExists);
        }
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        if nodes.contains_key(path) {
            return Err(FsError::AlreadyExists);
        }
        let parent = path.parent().ok_or(FsError::AlreadyExists)?;
        match nodes.get(&parent) {
            Some(node) if node.is_directory() => {}
            Some(_) => return Err(FsError::NotDirectory),
            None => return Err(FsError::NotFound),
        }
        nodes.insert(path.clone(), Node::dir(DEFAULT_DIR_MODE));
        Ok(())
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if node.is_directory() {
            return Err(FsError::IsDirectory);
        }
        nodes.remove(path);
        Ok(())
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if !node.is_directory() {
            return Err(FsError::NotDirectory);
        }
        if direct_children(&nodes, path).next().is_some() {
            return Err(FsError::NotEmpty);
        }
        nodes.remove(path);
        Ok(())
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        rename_node_tree(&mut nodes, old_path, new_path)
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
        node.set_times(accessed_time_ns, modified_time_ns);
        Ok(())
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
        node.set_permissions(permissions & PERMISSION_MODE_MASK);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
