use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::{
    File, FileSeekFrom, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

use super::{MemFs, Node};

#[derive(Debug)]
pub(super) struct MemFile {
    nodes: Arc<RwLock<BTreeMap<NormalizedPath, Node>>>,
    path: NormalizedPath,
    offset: usize,
    readable: bool,
    writable: bool,
}

impl MemFile {
    pub(super) fn new(
        nodes: Arc<RwLock<BTreeMap<NormalizedPath, Node>>>,
        path: NormalizedPath,
        options: OpenOptions,
    ) -> Self {
        Self {
            nodes,
            path,
            offset: 0,
            readable: options.read,
            writable: options.write,
        }
    }
}

impl File for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.readable {
            return Err(FsError::PermissionDenied);
        }
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
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
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
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
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
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
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        let node = nodes.get_mut(&self.path).ok_or(FsError::NotFound)?;
        if node.kind == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        node.data.resize(len, 0);
        Ok(())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        let nodes = self
            .nodes
            .read()
            .map_err(|_| FsError::Other("memfs lock poisoned".to_owned()))?;
        MemFs::metadata_for(&nodes, &self.path)
    }
}
