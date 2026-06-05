use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::{TermDevice, file_metadata, modes};

mod queue;
mod stream;
mod winch;

pub(crate) use queue::read_from_queue;
pub(crate) use stream::TermFile;
pub(crate) use winch::WinchFile;

pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read {
        return Err(FsError::PermissionDenied);
    }
    if options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

pub(crate) fn require_file_open(options: OpenOptions) -> FsResult<()> {
    if !options.read && !options.write {
        return Err(FsError::PermissionDenied);
    }
    if options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

pub(crate) fn access(options: OpenOptions) -> FsResult<FileAccess> {
    if !options.read && !options.write {
        return Err(FsError::PermissionDenied);
    }
    if options.create {
        return Err(FsError::PermissionDenied);
    }
    Ok(FileAccess {
        read: options.read,
        write: options.write,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FileAccess {
    read: bool,
    write: bool,
}

impl FileAccess {
    fn can_read(self) -> FsResult<()> {
        if self.read {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    fn can_write(self) -> FsResult<()> {
        if self.write {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }
}

#[derive(Debug)]
pub(crate) struct NewTermFile {
    device: TermDevice,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewTermFile {
    pub(crate) fn new(device: TermDevice) -> Self {
        Self {
            device,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewTermFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.device.alloc()?;
            self.bytes = Some(format!("{id}\n").into_bytes());
        }
        read_from_slice(
            self.bytes
                .as_ref()
                .expect("new term bytes are initialized before reading"),
            &mut self.offset,
            buf,
        )
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::READ_ONLY_FILE))
    }
}

#[derive(Debug)]
pub(crate) struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl BytesFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        read_from_slice(&self.bytes, &mut self.offset, buf)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(
            self.bytes.len() as u64,
            modes::READ_ONLY_FILE,
        ))
    }
}

#[derive(Debug)]
pub(crate) struct ControlFile {
    device: TermDevice,
    id: String,
    access: FileAccess,
    data: Vec<u8>,
}

impl ControlFile {
    pub(crate) fn new(device: TermDevice, id: String, access: FileAccess) -> Self {
        Self {
            device,
            id,
            access,
            data: Vec::new(),
        }
    }
}

impl File for ControlFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.data.extend_from_slice(buf);
        let command = String::from_utf8_lossy(&self.data).trim().to_owned();
        if command.is_empty() {
            return Ok(buf.len());
        }
        if "close".starts_with(command.as_str()) {
            if command == "close" {
                self.device.close(&self.id)?;
                self.data.clear();
            }
            return Ok(buf.len());
        }
        Err(FsError::NotSupported)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::CONTROL_FILE))
    }
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    Ok(len)
}
