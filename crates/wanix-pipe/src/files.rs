use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::channel::PipeChannel;
use crate::{PipeDevice, file_metadata, modes};

pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read {
        return Err(FsError::PermissionDenied);
    }
    if options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    Ok(len)
}

/// `#pipe/new`: allocates a channel on first read and returns its id.
#[derive(Debug)]
pub(crate) struct NewPipeFile {
    device: PipeDevice,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewPipeFile {
    pub(crate) fn new(device: PipeDevice) -> Self {
        Self {
            device,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewPipeFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.device.alloc()?;
            self.bytes = Some(format!("{id}\n").into_bytes());
        }
        let Some(bytes) = self.bytes.as_ref() else {
            return Err(FsError::Other("pipe new bytes missing".to_owned()));
        };
        let mut offset = self.offset;
        let read = read_from_slice(bytes, &mut offset, buf)?;
        self.offset = offset;
        Ok(read)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::READ_ONLY_FILE))
    }
}

/// `#pipe/<id>/id`: serves a fixed snapshot of the channel id.
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

/// The read end of a pipe: drains buffered bytes, blocking until data or EOF.
/// Dropping the last read end breaks the pipe for writers (Unix `EPIPE`).
pub(crate) struct PipeReader {
    channel: Arc<PipeChannel>,
}

impl PipeReader {
    pub(crate) fn new(channel: Arc<PipeChannel>) -> FsResult<Self> {
        channel.add_reader()?;
        Ok(Self { channel })
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.channel.close_reader();
    }
}

impl File for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.channel.read(buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.channel.read_ready()
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

/// The write end of a pipe: appends bytes and signals EOF on drop.
pub(crate) struct PipeWriter {
    channel: Arc<PipeChannel>,
}

impl PipeWriter {
    pub(crate) fn new(channel: Arc<PipeChannel>) -> FsResult<Self> {
        channel.add_writer()?;
        Ok(Self { channel })
    }
}

impl File for PipeWriter {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.channel.write(buf)
    }

    fn write_ready(&self) -> FsResult<bool> {
        self.channel.write_ready()
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.channel.close_writer();
    }
}
