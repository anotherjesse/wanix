use wanix_fs::{DirEntry, File, FileSeekFrom, FileType, FsError, FsResult, Metadata};

use crate::OpenFile;

mod control;
mod field;
mod new_task;
mod wait;

pub(crate) use control::ControlFile;
pub(crate) use field::{Field, FieldFile, field_metadata};
pub(crate) use new_task::NewTaskFile;
pub(crate) use wait::WaitFile;

pub(crate) const TASK_FILE_READ_ONLY_MODE: u32 = 0o555;
pub(crate) const TASK_FILE_READ_WRITE_MODE: u32 = 0o755;
pub(crate) const TASK_FD_FILE_MODE: u32 = 0o666;
pub(crate) const TASK_DIRECTORY_MODE: u32 = TASK_FILE_READ_WRITE_MODE;

#[derive(Debug, Clone, Copy)]
pub(crate) struct FileAccess {
    read: bool,
    write: bool,
}

impl FileAccess {
    pub(crate) fn new(read: bool, write: bool) -> Self {
        Self { read, write }
    }

    pub(crate) fn read_only() -> Self {
        Self::new(true, false)
    }

    pub(crate) fn can_read(self) -> FsResult<()> {
        if self.read {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(crate) fn can_write(self) -> FsResult<()> {
        if self.write {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(crate) fn can_seek(self) -> FsResult<()> {
        if self.read || self.write {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }
}

#[derive(Debug)]
pub(crate) struct FdProxyFile {
    file: OpenFile,
    access: FileAccess,
}

impl FdProxyFile {
    pub(crate) fn new(file: OpenFile, access: FileAccess) -> Self {
        Self { file, access }
    }
}

impl File for FdProxyFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        self.file.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.file.write(buf)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.access.can_seek()?;
        self.file.seek(from)
    }

    fn tell(&self) -> FsResult<u64> {
        self.access.can_seek()?;
        self.file.tell()
    }

    fn is_seekable(&self) -> bool {
        self.file.is_seekable().unwrap_or(false)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.file.metadata()
    }
}

pub(crate) fn task_entries() -> Vec<DirEntry> {
    [
        ("cmd", file_metadata(0, TASK_FILE_READ_WRITE_MODE)),
        ("ctl", file_metadata(0, TASK_FILE_READ_WRITE_MODE)),
        ("dir", file_metadata(2, TASK_FILE_READ_WRITE_MODE)),
        ("env", file_metadata(1, TASK_FILE_READ_WRITE_MODE)),
        ("exit", file_metadata(1, TASK_FILE_READ_WRITE_MODE)),
        ("fd", directory_metadata()),
        ("id", file_metadata(2, TASK_FILE_READ_ONLY_MODE)),
        ("kind", file_metadata(0, TASK_FILE_READ_ONLY_MODE)),
        ("wait", file_metadata(0, TASK_FILE_READ_ONLY_MODE)),
    ]
    .into_iter()
    .map(|(name, metadata)| DirEntry::new(name, metadata))
    .collect()
}

pub(crate) fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, TASK_DIRECTORY_MODE)
}

pub(crate) fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

fn read_from_slice(data: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let available = data.len().saturating_sub(*offset);
    let count = available.min(buf.len());
    buf[..count].copy_from_slice(&data[*offset..*offset + count]);
    *offset += count;
    Ok(count)
}

fn seek_offset(current: usize, len: usize, from: FileSeekFrom) -> FsResult<usize> {
    let base = match from {
        FileSeekFrom::Start(offset) => {
            return usize::try_from(offset).map_err(|_| FsError::InvalidOffset);
        }
        FileSeekFrom::Current(_) => current as i128,
        FileSeekFrom::End(_) => len as i128,
    };
    let delta = match from {
        FileSeekFrom::Start(_) => 0,
        FileSeekFrom::Current(offset) | FileSeekFrom::End(offset) => offset as i128,
    };
    let next = base + delta;
    if next < 0 || next > usize::MAX as i128 {
        return Err(FsError::InvalidOffset);
    }
    Ok(next as usize)
}
