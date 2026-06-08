//! Read/write file handles for the `#sites` control files.

use std::mem;

use wanix_fs::{File, FsError, FsResult, Metadata};

use crate::{Bindings, Host, SiteSource, file_metadata};

/// A snapshot-on-open read handle over a host binding's control bytes.
#[derive(Debug)]
pub(crate) struct SitesReadFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl SitesReadFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for SitesReadFile {
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

/// A buffer-and-commit-on-close write handle: the buffered descriptor replaces
/// the host's binding source when the handle is dropped. The host is created
/// eagerly at open time (as a placeholder source) so a stat between open and
/// close still finds it, mirroring the `#kv` ensure-key pattern.
#[derive(Debug)]
pub(crate) struct SitesWriteFile {
    bindings: Bindings,
    host: Host,
    buffer: Vec<u8>,
}

impl SitesWriteFile {
    pub(crate) fn new(bindings: Bindings, host: Host) -> Self {
        Self {
            bindings,
            host,
            buffer: Vec::new(),
        }
    }
}

impl File for SitesWriteFile {
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

impl Drop for SitesWriteFile {
    fn drop(&mut self) {
        let descriptor = String::from_utf8_lossy(&mem::take(&mut self.buffer)).into_owned();
        let Some(source) = SiteSource::parse_descriptor(&descriptor) else {
            return;
        };
        if let Ok(mut bindings) = self.bindings.write() {
            bindings.insert(self.host.clone(), source);
        }
    }
}
