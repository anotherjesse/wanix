//! `File` implementations for the ToolFS surface.

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::fs::{file_metadata, modes};
use crate::principal::ToolPrincipal;
use crate::service::{JobField, ToolService};

/// Root metadata files and job snapshots are read-only opens.
pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read || options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

/// A read-only byte snapshot captured at open time, so concurrent job
/// progress cannot tear an in-flight read.
#[derive(Debug)]
pub(crate) struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
    mode: u32,
}

impl BytesFile {
    pub(crate) fn new(bytes: Vec<u8>, mode: u32) -> Self {
        Self {
            bytes,
            offset: 0,
            mode,
        }
    }
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        Ok(read_from_slice(&self.bytes, &mut self.offset, buf))
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(self.bytes.len() as u64, self.mode))
    }
}

/// The `new` file: the first read allocates a job for this view's principal
/// and returns its opaque id; subsequent reads drain the same id bytes.
pub(crate) struct NewJobFile {
    service: ToolService,
    principal: ToolPrincipal,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewJobFile {
    pub(crate) fn new(service: ToolService, principal: ToolPrincipal) -> Self {
        Self {
            service,
            principal,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewJobFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.service.alloc(&self.principal)?;
            self.bytes = Some(format!("{id}\n").into_bytes());
        }
        let bytes = self.bytes.as_deref().unwrap_or_default();
        Ok(read_from_slice(bytes, &mut self.offset, buf))
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::READ_FILE))
    }
}

/// Write handle for `in` and `params.json`: every write lands in the job
/// record immediately (params parse at write time inside the service).
pub(crate) struct AppendFile {
    service: ToolService,
    principal: ToolPrincipal,
    id: String,
    field: JobField,
}

impl AppendFile {
    pub(crate) fn new(
        service: ToolService,
        principal: ToolPrincipal,
        id: String,
        field: JobField,
    ) -> Self {
        Self {
            service,
            principal,
            id,
            field,
        }
    }
}

impl File for AppendFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        match self.field {
            JobField::In => self.service.append_input(&self.principal, &self.id, buf),
            JobField::Params => self.service.append_params(&self.principal, &self.id, buf),
            _ => Err(FsError::NotSupported),
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        let len = self
            .service
            .field_len(&self.principal, &self.id, self.field)?;
        Ok(file_metadata(len, modes::DATA_FILE))
    }
}

/// The job `ctl` file: buffers writes and dispatches `run`/`abort`/`close`.
pub(crate) struct CtlFile {
    service: ToolService,
    principal: ToolPrincipal,
    id: String,
    buffer: Vec<u8>,
}

impl CtlFile {
    pub(crate) fn new(service: ToolService, principal: ToolPrincipal, id: String) -> Self {
        Self {
            service,
            principal,
            id,
            buffer: Vec::new(),
        }
    }
}

impl File for CtlFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer.extend_from_slice(buf);
        let command = String::from_utf8_lossy(&self.buffer).trim().to_owned();
        if command.is_empty() {
            return Ok(buf.len());
        }
        match command.as_str() {
            "run" => self.service.run(&self.principal, &self.id)?,
            "abort" => self.service.abort(&self.principal, &self.id)?,
            "close" => self.service.close(&self.principal, &self.id)?,
            verb => return Err(FsError::Other(format!("unknown ctl verb {verb:?}"))),
        }
        self.buffer.clear();
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::CTL_FILE))
    }
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> usize {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    len
}
