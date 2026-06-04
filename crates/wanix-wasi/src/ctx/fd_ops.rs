use wanix_fs::{DirEntry, FileSystem};

use crate::{
    Errno, FileStat, WasiCtx, WasiFd, WasiFdStat, WasiFile, WasiFileType, WasiOpenOptions,
    WasiPrestat, WasiRights,
};

use super::handle::Handle;
use super::seek::{WasiWhence, seek_handle, tell_handle};

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

    /// Returns stat data for an open fd.
    pub fn fd_filestat_get(&self, fd: WasiFd) -> Result<FileStat, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => file.metadata_file().map(FileStat::new).map_err(Errno::from),
            Handle::Preopen { source_path, .. } => self.stat_path(source_path),
            Handle::Directory {
                path, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_FILESTAT_GET) {
                    return Err(Errno::Notcapable);
                }
                self.stat_path(path)
            }
            Handle::File {
                file, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_FILESTAT_GET) {
                    return Err(Errno::Notcapable);
                }
                file.metadata_file().map(FileStat::new).map_err(Errno::from)
            }
        }
    }

    /// Sets access and modification times for an open fd.
    pub fn fd_filestat_set_times(
        &self,
        fd: WasiFd,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        fstflags: u16,
    ) -> Result<(), Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { .. } => Err(Errno::Notcapable),
            Handle::Preopen { source_path, .. } => {
                self.set_path_times(source_path, accessed_time_ns, modified_time_ns, fstflags)
            }
            Handle::Directory {
                path, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_FILESTAT_SET_TIMES) {
                    return Err(Errno::Notcapable);
                }
                self.set_path_times(path, accessed_time_ns, modified_time_ns, fstflags)
            }
            Handle::File {
                path, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_FILESTAT_SET_TIMES) {
                    return Err(Errno::Notcapable);
                }
                self.set_path_times(path, accessed_time_ns, modified_time_ns, fstflags)
            }
        }
    }

    /// Sets the size for an open regular-file fd.
    pub fn fd_filestat_set_size(&self, fd: WasiFd, size: u64) -> Result<(), Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::File {
                file,
                write,
                rights_base,
                ..
            } => {
                if !*write || !rights_base.contains(WasiRights::FD_FILESTAT_SET_SIZE) {
                    return Err(Errno::Notcapable);
                }
                file.set_len_file(size).map_err(Errno::from)
            }
            Handle::Stdio { .. } | Handle::Preopen { .. } | Handle::Directory { .. } => {
                Err(Errno::Notcapable)
            }
        }
    }

    /// Returns prestat data for a preopened directory fd.
    pub fn fd_prestat_get(&self, fd: WasiFd) -> Result<WasiPrestat, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Preopen { guest_path, .. } => Ok(WasiPrestat::from_path(guest_path)),
            Handle::Stdio { .. } | Handle::Directory { .. } | Handle::File { .. } => {
                Err(Errno::Badf)
            }
        }
    }

    /// Copies the preopened directory name into `dst`.
    pub fn fd_prestat_dir_name(&self, fd: WasiFd, dst: &mut [u8]) -> Result<usize, Errno> {
        let prestat = self.fd_prestat_get(fd)?;
        let name = prestat.dir_name().as_bytes();
        if dst.len() < name.len() {
            return Err(Errno::Inval);
        }
        dst[..name.len()].copy_from_slice(name);
        Ok(name.len())
    }

    /// Returns fdstat data for an open fd.
    pub fn fd_fdstat_get(&self, fd: WasiFd) -> Result<WasiFdStat, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => Ok(WasiFdStat::new(
                WasiFileType::CharacterDevice,
                attached_file_rights(file)?,
                WasiRights::NONE,
            )),
            Handle::Preopen { .. } => Ok(WasiFdStat::new(
                WasiFileType::Directory,
                WasiRights::DIRECTORY_BASE,
                WasiRights::DIRECTORY_INHERITING,
            )),
            Handle::Directory {
                rights_base,
                rights_inheriting,
                ..
            } => Ok(WasiFdStat::new(
                WasiFileType::Directory,
                *rights_base,
                *rights_inheriting,
            )),
            Handle::File {
                rights_base,
                fdflags,
                ..
            } => Ok(WasiFdStat::new_with_fdflags(
                WasiFileType::RegularFile,
                *fdflags,
                *rights_base,
                WasiRights::NONE,
            )),
        }
    }

    /// Updates mutable Preview 1 fdflags for an open fd.
    pub fn fd_fdstat_set_flags(&mut self, fd: WasiFd, fdflags: u16) -> Result<(), Errno> {
        WasiOpenOptions::validate_preview1_fdflags(fdflags)?;
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::File {
                file,
                write,
                rights_base,
                fdflags: current_fdflags,
                ..
            } => {
                let append = fdflags & WasiOpenOptions::FDFLAGS_APPEND != 0;
                if append && (!*write || !rights_base.contains(WasiRights::FD_WRITE)) {
                    return Err(Errno::Notcapable);
                }
                file.set_append(append).map_err(Errno::from)?;
                *current_fdflags = fdflags;
                Ok(())
            }
            Handle::Stdio { .. } | Handle::Preopen { .. } | Handle::Directory { .. } => {
                if fdflags == 0 {
                    Ok(())
                } else {
                    Err(Errno::Notcapable)
                }
            }
        }
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

fn attached_file_rights(file: &WasiFile) -> Result<WasiRights, Errno> {
    let mut rights = WasiRights::FD_FILESTAT_GET;
    if file.can_read() {
        rights |= WasiRights::FD_READ;
    }
    if file.can_write() {
        rights |= WasiRights::FD_WRITE;
    }
    if file.is_seekable_file().map_err(Errno::from)? {
        rights |= WasiRights::FD_SEEK | WasiRights::FD_TELL;
    }
    Ok(rights)
}
