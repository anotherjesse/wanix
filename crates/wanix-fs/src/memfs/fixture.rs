use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::{FsError, FsResult, NormalizedPath};

use super::{
    MemFs,
    node::{DEFAULT_DIR_MODE, DEFAULT_FILE_MODE, Node},
};

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
}
