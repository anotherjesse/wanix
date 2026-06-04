use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata};

use crate::WasiFileAccess;

/// Shared file handle attached to a WASI fd.
#[derive(Clone)]
pub struct WasiFile {
    file: Arc<Mutex<Box<dyn File>>>,
    label: String,
    access: Arc<Mutex<WasiFileAccess>>,
}

impl fmt::Debug for WasiFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let access = self.access.lock().map(|access| *access).ok();
        f.debug_struct("WasiFile")
            .field("label", &self.label)
            .field("access", &access)
            .finish_non_exhaustive()
    }
}

impl WasiFile {
    /// Creates a shared WASI fd file attachment.
    pub fn new(file: Box<dyn File>, label: impl Into<String>, access: WasiFileAccess) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
            label: label.into(),
            access: Arc::new(Mutex::new(access)),
        }
    }

    /// Creates a read-only stdin attachment.
    #[must_use]
    pub fn stdin(file: Box<dyn File>, label: impl Into<String>) -> Self {
        Self::new(file, label, WasiFileAccess::read_only())
    }

    /// Creates a write-only stdout/stderr attachment.
    #[must_use]
    pub fn output(file: Box<dyn File>, label: impl Into<String>) -> Self {
        Self::new(file, label, WasiFileAccess::write_only())
    }

    /// Returns the debug label associated with this attached file.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn can_read(&self) -> bool {
        self.access().is_ok_and(WasiFileAccess::can_read)
    }

    pub(crate) fn can_write(&self) -> bool {
        self.access().is_ok_and(WasiFileAccess::can_write)
    }

    pub(crate) fn set_append(&self, append: bool) -> FsResult<()> {
        let mut access = self.access.lock().map_err(access_lock_poisoned)?;
        *access = access.with_append(append);
        Ok(())
    }

    pub(crate) fn read_bytes(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .read(buf)
    }

    pub(crate) fn read_ready_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .read_ready()
    }

    pub(crate) fn write_bytes(&self, buf: &[u8]) -> FsResult<usize> {
        let append = self.access()?.append();
        let mut file = self
            .file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?;
        if append {
            file.seek(FileSeekFrom::End(0))?;
        }
        file.write(buf)
    }

    pub(crate) fn write_ready_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .write_ready()
    }

    pub(crate) fn seek_file(&self, from: FileSeekFrom) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .seek(from)
    }

    pub(crate) fn tell_file(&self) -> FsResult<u64> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .tell()
    }

    pub(crate) fn is_seekable_file(&self) -> FsResult<bool> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))
            .map(|file| file.is_seekable())
    }

    pub(crate) fn metadata_file(&self) -> FsResult<Metadata> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .metadata()
    }

    pub(crate) fn set_len_file(&self, len: u64) -> FsResult<()> {
        self.file
            .lock()
            .map_err(|_| FsError::Other("WASI fd file lock poisoned".to_owned()))?
            .set_len(len)
    }

    fn access(&self) -> FsResult<WasiFileAccess> {
        self.access
            .lock()
            .map(|access| *access)
            .map_err(access_lock_poisoned)
    }
}

fn access_lock_poisoned<T>(_: T) -> FsError {
    FsError::Other("WASI fd access lock poisoned".to_owned())
}

impl File for WasiFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.can_read() {
            return Err(FsError::PermissionDenied);
        }
        self.read_bytes(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.write_bytes(buf)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.seek_file(from)
    }

    fn tell(&self) -> FsResult<u64> {
        self.tell_file()
    }

    fn is_seekable(&self) -> bool {
        self.is_seekable_file().unwrap_or(false)
    }

    fn read_ready(&self) -> FsResult<bool> {
        if !self.can_read() {
            return Err(FsError::PermissionDenied);
        }
        self.read_ready_file()
    }

    fn write_ready(&self) -> FsResult<bool> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.write_ready_file()
    }

    fn set_len(&mut self, len: u64) -> FsResult<()> {
        if !self.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.set_len_file(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.metadata_file()
    }
}
