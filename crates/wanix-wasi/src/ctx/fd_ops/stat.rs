use crate::{
    Errno, FileStat, WasiCtx, WasiFd, WasiFdStat, WasiFile, WasiFileType, WasiOpenOptions,
    WasiPrestat, WasiRights,
};

use super::super::handle::Handle;

impl WasiCtx {
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
