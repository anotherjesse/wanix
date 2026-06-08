//! [`CasBytesFile`]: a read-only in-memory cursor over a fetched blob.

use wanix_fs::{File, FileSeekFrom, FileType, FsError, FsResult, Metadata};

/// A read-only file handle backed by a blob already fetched from the store.
///
/// `CasSiteFs::open` resolves a path to a manifest entry, fetches the blob once,
/// and hands back this cursor. Writes are refused — a frozen site is immutable.
pub(crate) struct CasBytesFile {
    bytes: Vec<u8>,
    pos: usize,
    mode: u32,
}

impl CasBytesFile {
    /// Wraps already-fetched blob `bytes` with the given Unix mode bits.
    pub(crate) fn new(bytes: Vec<u8>, mode: u32) -> Self {
        Self {
            bytes,
            pos: 0,
            mode,
        }
    }
}

impl File for CasBytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let remaining = self.bytes.len().saturating_sub(self.pos);
        let take = remaining.min(buf.len());
        buf[..take].copy_from_slice(&self.bytes[self.pos..self.pos + take]);
        self.pos += take;
        Ok(take)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let len = self.bytes.len() as i64;
        let target = match from {
            FileSeekFrom::Start(offset) => offset as i64,
            FileSeekFrom::Current(delta) => self.pos as i64 + delta,
            FileSeekFrom::End(delta) => len + delta,
        };
        if target < 0 {
            return Err(FsError::InvalidPath("negative seek offset".to_owned()));
        }
        self.pos = target as usize;
        Ok(self.pos as u64)
    }

    fn tell(&self) -> FsResult<u64> {
        Ok(self.pos as u64)
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(
            FileType::File,
            self.bytes.len() as u64,
            self.mode,
        ))
    }
}
