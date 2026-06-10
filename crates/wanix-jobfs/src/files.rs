//! `File` implementations for the job-protocol surface, shared by every job
//! device's `FileSystem` (the device owns its path layout; these own the
//! per-file read/write discipline).

use std::sync::Arc;

use wanix_fs::{File, FileType, FsError, FsResult, LineBuffer, Metadata, OpenOptions};

use crate::jobs::{JobCore, JobField};
use crate::principal::JobPrincipal;

/// Conventional permission modes for the job-protocol file shapes.
pub mod modes {
    /// Job and root directories.
    pub const DIRECTORY: u32 = 0o555;
    /// Read-only snapshots and streams (`out`, `status`, `events`, …).
    pub const READ_FILE: u32 = 0o444;
    /// Request files written before run (`in`, `params.json`).
    pub const DATA_FILE: u32 = 0o666;
    /// The write-only `ctl` file.
    pub const CTL_FILE: u32 = 0o222;
}

/// Directory metadata for the fixed job-protocol directory shapes.
#[must_use]
pub fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

/// File metadata in the job-protocol shape.
#[must_use]
pub fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

/// Rejects any open that is not strictly read-only (root metadata files and
/// job snapshots).
///
/// # Errors
///
/// Returns `PermissionDenied` for write/create/truncate opens.
pub fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read || options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

/// A read-only byte snapshot captured at open time, so concurrent job
/// progress cannot tear an in-flight read.
#[derive(Debug)]
pub struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
    mode: u32,
}

impl BytesFile {
    /// A snapshot file over `bytes` with permission `mode`.
    #[must_use]
    pub fn new(bytes: Vec<u8>, mode: u32) -> Self {
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
pub struct NewJobFile {
    core: JobCore,
    principal: JobPrincipal,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewJobFile {
    /// An allocation handle for `principal` on `core`.
    #[must_use]
    pub fn new(core: JobCore, principal: JobPrincipal) -> Self {
        Self {
            core,
            principal,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewJobFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.core.alloc(&self.principal)?;
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
/// record immediately (params parse at write time inside the core).
pub struct AppendFile {
    core: JobCore,
    principal: JobPrincipal,
    id: String,
    field: JobField,
}

impl AppendFile {
    /// A write handle for one request field of job `id`.
    #[must_use]
    pub fn new(core: JobCore, principal: JobPrincipal, id: String, field: JobField) -> Self {
        Self {
            core,
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
            JobField::In => self.core.append_input(&self.principal, &self.id, buf),
            JobField::Params => self.core.append_params(&self.principal, &self.id, buf),
            _ => Err(FsError::NotSupported),
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        let len = self.core.field_len(&self.principal, &self.id, self.field)?;
        Ok(file_metadata(len, modes::DATA_FILE))
    }
}

/// The job `ctl` file: buffers writes and dispatches `run`/`abort`/`close`.
pub struct CtlFile {
    core: JobCore,
    principal: JobPrincipal,
    id: String,
    buffer: Vec<u8>,
}

impl CtlFile {
    /// A control handle for job `id`.
    #[must_use]
    pub fn new(core: JobCore, principal: JobPrincipal, id: String) -> Self {
        Self {
            core,
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
            "run" => self.core.run(&self.principal, &self.id)?,
            "abort" => self.core.abort(&self.principal, &self.id)?,
            "close" => self.core.close(&self.principal, &self.id)?,
            verb => return Err(FsError::Other(format!("unknown ctl verb {verb:?}"))),
        }
        self.buffer.clear();
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::CTL_FILE))
    }
}

/// The job `events` stream: a blocking, never-EOF read over the job's
/// bounded progress buffer. The buffer `Arc` is captured at open time
/// ([`JobCore::events_handle`]), so reads never touch the job-table lock;
/// EOF arrives only when the job finishes or is dropped.
pub struct EventsFile {
    buffer: Arc<LineBuffer>,
}

impl EventsFile {
    /// A stream reader over one job's events buffer.
    #[must_use]
    pub fn new(buffer: Arc<LineBuffer>) -> Self {
        Self { buffer }
    }
}

impl File for EventsFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.buffer.read(buf)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::READ_FILE))
    }
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> usize {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    len
}
