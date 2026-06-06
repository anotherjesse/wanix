use std::mem;

use wanix_fs::{File, FsError, FsResult, Metadata};

use crate::{Store, file_metadata};

/// A snapshot-on-open read handle: serves the value bytes captured when the
/// file was opened, so a concurrent overwrite cannot tear an in-flight read.
#[derive(Debug)]
pub(crate) struct KvReadFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl KvReadFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for KvReadFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        let len = remaining.min(buf.len());
        buf[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
        self.offset += len;
        Ok(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(self.bytes.len() as u64))
    }
}

/// A buffer-and-commit-on-close write handle: buffered writes replace the
/// key's value wholesale when the handle is dropped. The key is created
/// eagerly at open time so a stat between open and close still finds it.
#[derive(Debug)]
pub(crate) struct KvWriteFile {
    store: Store,
    key: String,
    buffer: Vec<u8>,
}

impl KvWriteFile {
    pub(crate) fn new(store: Store, key: String) -> Self {
        Self {
            store,
            key,
            buffer: Vec::new(),
        }
    }
}

impl File for KvWriteFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(self.buffer.len() as u64))
    }
}

impl Drop for KvWriteFile {
    fn drop(&mut self) {
        if let Ok(mut map) = self.store.write() {
            map.insert(self.key.clone(), mem::take(&mut self.buffer));
        }
    }
}
