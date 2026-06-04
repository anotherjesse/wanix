use wanix_fs::FileSeekFrom;

use crate::{Errno, WasiFile, WasiRights};

use super::handle::Handle;

/// WASI Preview 1 seek origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasiWhence {
    /// Seek relative to the start.
    Set,
    /// Seek relative to the current offset.
    Cur,
    /// Seek relative to the end.
    End,
}

impl WasiWhence {
    /// Converts a WASI Preview 1 whence code into a typed value.
    pub fn from_preview1(code: i32) -> Result<Self, Errno> {
        match code {
            0 => Ok(Self::Set),
            1 => Ok(Self::Cur),
            2 => Ok(Self::End),
            _ => Err(Errno::Inval),
        }
    }
}

pub(super) fn seek_handle(
    handle: &mut Handle,
    offset: i64,
    whence: WasiWhence,
) -> Result<u64, Errno> {
    match handle {
        Handle::Stdio { file } => seek_file(file, offset, whence),
        Handle::File {
            file, rights_base, ..
        } => {
            require_right(*rights_base, WasiRights::FD_SEEK)?;
            seek_file(file, offset, whence)
        }
        Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Notcapable),
    }
}

pub(super) fn tell_handle(handle: &Handle) -> Result<u64, Errno> {
    match handle {
        Handle::Stdio { file } => tell_file(file),
        Handle::File {
            file, rights_base, ..
        } => {
            require_right(*rights_base, WasiRights::FD_TELL)?;
            tell_file(file)
        }
        Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Notcapable),
    }
}

fn seek_file(file: &mut WasiFile, offset: i64, whence: WasiWhence) -> Result<u64, Errno> {
    require_seekable(file)?;
    let from = file_seek_from(offset, whence)?;
    file.seek_file(from).map_err(Errno::from)
}

fn tell_file(file: &WasiFile) -> Result<u64, Errno> {
    require_seekable(file)?;
    file.tell_file().map_err(Errno::from)
}

fn require_right(rights_base: WasiRights, required: WasiRights) -> Result<(), Errno> {
    if rights_base.contains(required) {
        Ok(())
    } else {
        Err(Errno::Notcapable)
    }
}

fn require_seekable(file: &WasiFile) -> Result<(), Errno> {
    if file.is_seekable_file().map_err(Errno::from)? {
        Ok(())
    } else {
        Err(Errno::Notcapable)
    }
}

fn file_seek_from(offset: i64, whence: WasiWhence) -> Result<FileSeekFrom, Errno> {
    match whence {
        WasiWhence::Set => {
            let offset = u64::try_from(offset).map_err(|_| Errno::Inval)?;
            Ok(FileSeekFrom::Start(offset))
        }
        WasiWhence::Cur => Ok(FileSeekFrom::Current(offset)),
        WasiWhence::End => Ok(FileSeekFrom::End(offset)),
    }
}
