use wanix_fs::{DirEntry, FileSystem};

use crate::{Errno, WasiCtx, WasiFd, WasiRights};

use super::handle::Handle;
use super::seek::{WasiWhence, seek_handle, tell_handle};

mod stat;

impl WasiCtx {
    /// Reads bytes from an open fd.
    pub fn fd_read(&mut self, fd: WasiFd, buf: &mut [u8]) -> Result<usize, Errno> {
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.can_read() {
                    return Err(Errno::Notcapable);
                }
                file.read_bytes(buf).map_err(Errno::from)
            }
            Handle::File {
                file,
                read,
                rights_base,
                ..
            } => {
                if !*read || !rights_base.contains(WasiRights::FD_READ) {
                    return Err(Errno::Notcapable);
                }
                file.read_bytes(buf).map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Writes bytes to an open fd.
    pub fn fd_write(&mut self, fd: WasiFd, buf: &[u8]) -> Result<usize, Errno> {
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.can_write() {
                    return Err(Errno::Notcapable);
                }
                file.write_bytes(buf).map_err(Errno::from)
            }
            Handle::File {
                file,
                write,
                rights_base,
                ..
            } => {
                if !*write || !rights_base.contains(WasiRights::FD_WRITE) {
                    return Err(Errno::Notcapable);
                }
                file.write_bytes(buf).map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Returns whether a nonblocking read on an open fd would produce data now.
    pub fn fd_read_ready(&self, fd: WasiFd) -> Result<bool, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.can_read() {
                    return Err(Errno::Notcapable);
                }
                file.read_ready_file().map_err(Errno::from)
            }
            Handle::File {
                file,
                read,
                rights_base,
                ..
            } => {
                if !*read || !rights_base.contains(WasiRights::FD_READ) {
                    return Err(Errno::Notcapable);
                }
                file.read_ready_file().map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Returns whether a nonblocking write on an open fd can be attempted now.
    pub fn fd_write_ready(&self, fd: WasiFd) -> Result<bool, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.can_write() {
                    return Err(Errno::Notcapable);
                }
                file.write_ready_file().map_err(Errno::from)
            }
            Handle::File {
                file,
                write,
                rights_base,
                ..
            } => {
                if !*write || !rights_base.contains(WasiRights::FD_WRITE) {
                    return Err(Errno::Notcapable);
                }
                file.write_ready_file().map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Closes a dynamic fd.
    pub fn fd_close(&mut self, fd: WasiFd) -> Result<(), Errno> {
        if fd.get() < self.next_dynamic_floor() {
            return Err(Errno::Badf);
        }
        let is_file = matches!(self.fds.get(&fd).ok_or(Errno::Badf)?, Handle::File { .. });
        if is_file && let Some(observer) = &self.fd_observer {
            observer.fd_closed(fd)?;
        }
        self.fds.remove(&fd).ok_or(Errno::Badf)?;
        Ok(())
    }

    /// Seeks an fd offset.
    pub fn fd_seek(&mut self, fd: WasiFd, offset: i64, whence: WasiWhence) -> Result<u64, Errno> {
        seek_handle(self.fds.get_mut(&fd).ok_or(Errno::Badf)?, offset, whence)
    }

    /// Returns the current fd offset.
    pub fn fd_tell(&self, fd: WasiFd) -> Result<u64, Errno> {
        tell_handle(self.fds.get(&fd).ok_or(Errno::Badf)?)
    }

    /// Reads a directory from an open preopen or directory fd.
    pub fn fd_read_dir(&self, fd: WasiFd) -> Result<Vec<DirEntry>, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Preopen { source_path, .. } => {
                self.namespace.read_dir(source_path).map_err(Errno::from)
            }
            Handle::Directory {
                path, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_READDIR) {
                    return Err(Errno::Notcapable);
                }
                self.namespace.read_dir(path).map_err(Errno::from)
            }
            Handle::Stdio { .. } | Handle::File { .. } => Err(Errno::Notdir),
        }
    }
}

impl Drop for WasiCtx {
    fn drop(&mut self) {
        let Some(observer) = &self.fd_observer else {
            return;
        };
        let dynamic_floor = self.next_dynamic_floor();
        for (fd, _handle) in self.fds.iter().filter(|(fd, handle)| {
            fd.get() >= dynamic_floor && matches!(handle, Handle::File { .. })
        }) {
            let _ = observer.fd_closed(*fd);
        }
    }
}
