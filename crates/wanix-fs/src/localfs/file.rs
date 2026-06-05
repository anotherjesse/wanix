use std::fs::File as StdFile;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::{File, FileSeekFrom, FsError, FsResult, Metadata, OpenOptions};

use super::{host::metadata_from_host, map_io_error};

#[derive(Debug)]
pub(super) struct LocalFile {
    file: StdFile,
    offset: u64,
    readable: bool,
    writable: bool,
}

impl LocalFile {
    pub(super) fn new(file: StdFile, options: OpenOptions) -> Self {
        Self {
            file,
            offset: 0,
            readable: options.read,
            writable: options.write,
        }
    }
}

impl File for LocalFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.readable {
            return Err(FsError::PermissionDenied);
        }
        let count = self.file.read(buf).map_err(map_io_error)?;
        self.offset = self
            .offset
            .checked_add(u64::try_from(count).map_err(|_| FsError::InvalidOffset)?)
            .ok_or(FsError::InvalidOffset)?;
        Ok(count)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let count = self.file.write(buf).map_err(map_io_error)?;
        self.offset = self
            .offset
            .checked_add(u64::try_from(count).map_err(|_| FsError::InvalidOffset)?)
            .ok_or(FsError::InvalidOffset)?;
        Ok(count)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let offset = self.file.seek(host_seek_from(from)).map_err(map_io_error)?;
        self.offset = offset;
        Ok(offset)
    }

    fn tell(&self) -> FsResult<u64> {
        Ok(self.offset)
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        self.file.set_len(len).map_err(map_io_error)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.file
            .metadata()
            .map(|metadata| metadata_from_host(&metadata))
            .map_err(map_io_error)
    }
}

fn host_seek_from(from: FileSeekFrom) -> SeekFrom {
    match from {
        FileSeekFrom::Start(offset) => SeekFrom::Start(offset),
        FileSeekFrom::Current(offset) => SeekFrom::Current(offset),
        FileSeekFrom::End(offset) => SeekFrom::End(offset),
    }
}
