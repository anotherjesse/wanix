use std::collections::BTreeMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{
    DirEntry, File, FileSeekFrom, FileSystem, FileType, FsError, FsResult, Metadata,
    NormalizedPath, OpenOptions,
};

/// In-memory filesystem used by the first Rust Wanix tests and demos.
#[derive(Debug, Clone)]
pub struct MemFs {
    nodes: Arc<RwLock<BTreeMap<NormalizedPath, Node>>>,
}

#[derive(Debug, Clone)]
struct Node {
    kind: FileType,
    mode: u32,
    data: Vec<u8>,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl Node {
    fn dir(mode: u32) -> Self {
        Self {
            kind: FileType::Directory,
            mode,
            data: Vec::new(),
            accessed_time_ns: 0,
            modified_time_ns: 0,
            changed_time_ns: 0,
        }
    }

    fn file(data: Vec<u8>, mode: u32) -> Self {
        Self {
            kind: FileType::File,
            mode,
            data,
            accessed_time_ns: 0,
            modified_time_ns: 0,
            changed_time_ns: 0,
        }
    }

    fn metadata(&self, len: u64) -> Metadata {
        Metadata::new_with_times(
            self.kind,
            len,
            self.mode,
            self.accessed_time_ns,
            self.modified_time_ns,
            self.changed_time_ns,
        )
    }
}

type Nodes = BTreeMap<NormalizedPath, Node>;

/// Acquires the shared node map for reading, mapping a poisoned lock to a
/// filesystem error.
fn read_nodes(lock: &RwLock<Nodes>) -> FsResult<RwLockReadGuard<'_, Nodes>> {
    lock.read()
        .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))
}

/// Acquires the shared node map for writing, mapping a poisoned lock to a
/// filesystem error.
fn write_nodes(lock: &RwLock<Nodes>) -> FsResult<RwLockWriteGuard<'_, Nodes>> {
    lock.write()
        .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))
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
        nodes.insert(
            NormalizedPath::new(".").expect("root path is valid"),
            Node::dir(0o755),
        );
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

        let mut nodes = write_nodes(&self.nodes)?;
        for dir in stack.into_iter().rev() {
            match nodes.get(&dir) {
                Some(node) if node.kind != FileType::Directory => {
                    return Err(FsError::NotDirectory);
                }
                Some(_) => {}
                None => {
                    nodes.insert(dir, Node::dir(0o755));
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

        let mut nodes = write_nodes(&self.nodes)?;
        if matches!(nodes.get(&path), Some(node) if node.kind == FileType::Directory) {
            return Err(FsError::IsDirectory);
        }
        nodes.insert(path, Node::file(data.as_ref().to_vec(), 0o644));
        Ok(())
    }

    /// Reads a complete file into memory.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file is missing or is a directory.
    pub fn read_file(&self, path: impl AsRef<str>) -> FsResult<Vec<u8>> {
        let path = NormalizedPath::new(path)?;
        let nodes = read_nodes(&self.nodes)?;
        let node = nodes.get(&path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        Ok(node.data.clone())
    }

    fn metadata_for(
        nodes: &BTreeMap<NormalizedPath, Node>,
        path: &NormalizedPath,
    ) -> FsResult<Metadata> {
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        let len = match node.kind {
            FileType::Directory => 2 + direct_children(nodes, path).count() as u64,
            FileType::File | FileType::Symlink => node.data.len() as u64,
        };
        Ok(node.metadata(len))
    }
}

impl FileSystem for MemFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        if options.create && !options.write {
            return Err(FsError::PermissionDenied);
        }
        if options.truncate && !options.write {
            return Err(FsError::PermissionDenied);
        }
        if path.as_str() == "." && (options.write || options.create || options.truncate) {
            return Err(FsError::IsDirectory);
        }

        {
            let mut nodes = write_nodes(&self.nodes)?;
            if options.create && !nodes.contains_key(path) {
                let parent = path.parent().ok_or(FsError::IsDirectory)?;
                match nodes.get(&parent) {
                    Some(node) if node.kind == FileType::Directory => {}
                    Some(_) => return Err(FsError::NotDirectory),
                    None => return Err(FsError::NotFound),
                }
                nodes.insert(path.clone(), Node::file(Vec::new(), 0o644));
            }

            let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
            if node.kind == FileType::Directory {
                return Err(FsError::IsDirectory);
            }
            if options.truncate {
                node.data.clear();
            }
        }

        Ok(Box::new(MemFile {
            nodes: Arc::clone(&self.nodes),
            path: path.clone(),
            offset: 0,
            readable: options.read,
            writable: options.write,
        }))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        let nodes = read_nodes(&self.nodes)?;
        Self::metadata_for(&nodes, path)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let nodes = read_nodes(&self.nodes)?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if node.kind != FileType::Directory {
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
        let mut nodes = write_nodes(&self.nodes)?;
        if nodes.contains_key(path) {
            return Err(FsError::AlreadyExists);
        }
        let parent = path.parent().ok_or(FsError::AlreadyExists)?;
        match nodes.get(&parent) {
            Some(node) if node.kind == FileType::Directory => {}
            Some(_) => return Err(FsError::NotDirectory),
            None => return Err(FsError::NotFound),
        }
        nodes.insert(path.clone(), Node::dir(0o755));
        Ok(())
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        let nodes = read_nodes(&self.nodes)?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if node.kind != FileType::Symlink {
            return Err(FsError::InvalidPath(format!("{path} is not a symlink")));
        }
        Ok(node.data.clone())
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::AlreadyExists);
        }
        let mut nodes = write_nodes(&self.nodes)?;
        if nodes.contains_key(path) {
            return Err(FsError::AlreadyExists);
        }
        let parent = path.parent().ok_or(FsError::AlreadyExists)?;
        match nodes.get(&parent) {
            Some(node) if node.kind == FileType::Directory => {}
            Some(_) => return Err(FsError::NotDirectory),
            None => return Err(FsError::NotFound),
        }
        let mut node = Node::file(target.to_vec(), 0o777);
        node.kind = FileType::Symlink;
        nodes.insert(path.clone(), node);
        Ok(())
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        nodes.remove(path);
        Ok(())
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get(path).ok_or(FsError::NotFound)?;
        if node.kind != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        if direct_children(&nodes, path).next().is_some() {
            return Err(FsError::NotEmpty);
        }
        nodes.remove(path);
        Ok(())
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        if old_path.as_str() == "." || new_path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }

        let mut nodes = write_nodes(&self.nodes)?;
        let old_node = nodes.get(old_path).cloned().ok_or(FsError::NotFound)?;
        if old_path == new_path {
            return Ok(());
        }
        if old_node.kind == FileType::Directory && is_descendant_path(new_path, old_path) {
            return Err(FsError::PermissionDenied);
        }
        let new_parent = new_path.parent().ok_or(FsError::PermissionDenied)?;
        match nodes.get(&new_parent) {
            Some(node) if node.kind == FileType::Directory => {}
            Some(_) => return Err(FsError::NotDirectory),
            None => return Err(FsError::NotFound),
        }

        if let Some(new_node) = nodes.get(new_path) {
            match (old_node.kind, new_node.kind) {
                (FileType::Directory, FileType::Directory)
                    if direct_children(&nodes, new_path).next().is_some() =>
                {
                    return Err(FsError::NotEmpty);
                }
                (FileType::Directory, FileType::Directory) => {}
                (FileType::Directory, _) => return Err(FsError::NotDirectory),
                (_, FileType::Directory) => return Err(FsError::IsDirectory),
                _ => {}
            }
        }
        nodes.remove(new_path);

        if old_node.kind == FileType::Directory {
            let moved = nodes
                .iter()
                .filter_map(|(path, node)| {
                    if path == old_path || is_descendant_path(path, old_path) {
                        let suffix = path
                            .as_str()
                            .strip_prefix(old_path.as_str())
                            .expect("descendant path starts with old path");
                        let next = NormalizedPath::new(format!("{new_path}{suffix}"))
                            .expect("renamed memfs path remains normalized");
                        Some((path.clone(), next, node.clone()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for (old, _, _) in &moved {
                nodes.remove(old);
            }
            for (_, new, node) in moved {
                nodes.insert(new, node);
            }
        } else {
            nodes.remove(old_path);
            nodes.insert(new_path.clone(), old_node);
        }
        Ok(())
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
        node.accessed_time_ns = accessed_time_ns;
        node.modified_time_ns = modified_time_ns;
        Ok(())
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
        node.mode = permissions & 0o7777;
        Ok(())
    }
}

#[derive(Debug)]
struct MemFile {
    nodes: Arc<RwLock<BTreeMap<NormalizedPath, Node>>>,
    path: NormalizedPath,
    offset: usize,
    readable: bool,
    writable: bool,
}

impl File for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.readable {
            return Err(FsError::PermissionDenied);
        }
        let nodes = read_nodes(&self.nodes)?;
        let node = nodes.get(&self.path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        let available = node.data.len().saturating_sub(self.offset);
        let count = available.min(buf.len());
        buf[..count].copy_from_slice(&node.data[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get_mut(&self.path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        let end = self
            .offset
            .checked_add(buf.len())
            .ok_or_else(|| FsError::Other("memfs write offset overflow".to_owned()))?;
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        node.data[self.offset..end].copy_from_slice(buf);
        self.offset = end;
        Ok(buf.len())
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let nodes = read_nodes(&self.nodes)?;
        let node = nodes.get(&self.path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        let current = i128::try_from(self.offset).map_err(|_| FsError::InvalidOffset)?;
        let end = i128::try_from(node.data.len()).map_err(|_| FsError::InvalidOffset)?;
        let next = match from {
            FileSeekFrom::Start(offset) => i128::from(offset),
            FileSeekFrom::Current(offset) => current + i128::from(offset),
            FileSeekFrom::End(offset) => end + i128::from(offset),
        };
        let next = usize::try_from(next).map_err(|_| FsError::InvalidOffset)?;
        self.offset = next;
        u64::try_from(next).map_err(|_| FsError::InvalidOffset)
    }

    fn tell(&self) -> FsResult<u64> {
        u64::try_from(self.offset).map_err(|_| FsError::InvalidOffset)
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let len = usize::try_from(len).map_err(|_| FsError::InvalidOffset)?;
        let mut nodes = write_nodes(&self.nodes)?;
        let node = nodes.get_mut(&self.path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        node.data.resize(len, 0);
        Ok(())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        let nodes = read_nodes(&self.nodes)?;
        MemFs::metadata_for(&nodes, &self.path)
    }
}

fn direct_children<'a>(
    nodes: &'a BTreeMap<NormalizedPath, Node>,
    path: &'a NormalizedPath,
) -> impl Iterator<Item = &'a NormalizedPath> {
    nodes.keys().filter(move |candidate| {
        if candidate.as_str() == "." {
            return false;
        }
        candidate.parent().as_ref() == Some(path)
    })
}

fn is_descendant_path(path: &NormalizedPath, ancestor: &NormalizedPath) -> bool {
    path.as_str()
        .strip_prefix(ancestor.as_str())
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests;
