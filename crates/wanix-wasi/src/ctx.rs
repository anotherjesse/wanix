use std::collections::BTreeMap;
use std::sync::Arc;

use wanix_fs::{DirEntry, FileSeekFrom, FileSystem, FileType, NormalizedPath, OpenOptions};
use wanix_vfs::Namespace;

use crate::{
    Errno, FileStat, WasiConfig, WasiFd, WasiFdObserver, WasiFdStat, WasiFile, WasiFileAccess,
    WasiFileType, WasiFilestatSetTimes, WasiLookupFlags, WasiOpenOptions, WasiPathOpen,
    WasiPrestat, WasiRights,
};

mod handle;
mod open;
mod path;

use handle::{Handle, OpenFileHandle};
use open::FileOpenRequest;
use path::{is_rooted_service_path, join_paths, wasi_path, wasi_symlink_target};

const FIRST_PREOPEN_FD: u32 = 3;

/// Host context for Wanix-backed WASI filesystem operations.
#[derive(Debug)]
pub struct WasiCtx {
    namespace: Namespace,
    fds: BTreeMap<WasiFd, Handle>,
    next_fd: u32,
    args: Vec<String>,
    env: Vec<String>,
    clock_time_ns: u64,
    fd_observer: Option<Arc<dyn WasiFdObserver>>,
}

impl WasiCtx {
    /// Creates a WASI context with fd 3 preopened at namespace root.
    #[must_use]
    pub fn new(config: WasiConfig) -> Self {
        Self::try_new(config).expect("WASI config preopens must be valid")
    }

    /// Creates a WASI context and validates all configured preopens.
    pub fn try_new(config: WasiConfig) -> Result<Self, Errno> {
        let mut fds = BTreeMap::new();
        for (fd, file) in config.stdio() {
            fds.insert(*fd, Handle::Stdio { file: file.clone() });
        }
        for (index, preopen) in config.preopens().iter().enumerate() {
            let source_path = preopen.source_path().clone();
            let metadata = config
                .namespace()
                .metadata(&source_path)
                .map_err(Errno::from)?;
            if metadata.file_type() != FileType::Directory {
                return Err(Errno::Notdir);
            }
            let fd =
                WasiFd::new(FIRST_PREOPEN_FD + u32::try_from(index).map_err(|_| Errno::Inval)?);
            fds.insert(
                fd,
                Handle::Preopen {
                    source_path,
                    guest_path: preopen.guest_path().clone(),
                },
            );
        }
        let next_fd =
            FIRST_PREOPEN_FD + u32::try_from(config.preopens().len()).map_err(|_| Errno::Inval)?;
        Ok(Self {
            namespace: config.namespace().clone(),
            fds,
            next_fd,
            args: config.args().to_vec(),
            env: config.env().to_vec(),
            clock_time_ns: config.clock_time_ns(),
            fd_observer: config.fd_observer(),
        })
    }

    /// Returns the namespace backing this WASI context.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Returns configured process arguments in WASI argv order.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns configured environment strings in `KEY=value` form.
    #[must_use]
    pub fn env(&self) -> &[String] {
        &self.env
    }

    /// Returns the number of currently open dynamic file descriptors.
    ///
    /// Standard descriptors and preopens are fixed process attachments; fds
    /// opened through `path_open` are dynamic and must be closed before a
    /// QuickJS VM snapshot can safely claim to contain all guest-visible state.
    #[must_use]
    pub fn open_dynamic_fd_count(&self) -> usize {
        let dynamic_floor = self.next_dynamic_floor();
        self.fds
            .keys()
            .filter(|fd| fd.get() >= dynamic_floor)
            .count()
    }

    /// Opens a namespace path relative to `dirfd`.
    pub fn path_open(
        &mut self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
        options: WasiOpenOptions,
    ) -> Result<WasiFd, Errno> {
        let (_, _, parent_rights_inheriting) = self.directory_handle(dirfd)?;
        let resolved = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_OPEN)?;
        self.open_resolved_path(resolved, options, None, parent_rights_inheriting)
    }

    /// Opens a namespace path using raw WASI Preview 1 `path_open` rights.
    pub fn path_open_preview1(
        &mut self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
        oflags: u16,
        rights_base: WasiRights,
        rights_inheriting: WasiRights,
        fdflags: u16,
    ) -> Result<WasiFd, Errno> {
        let (_, parent_rights_base, parent_rights_inheriting) = self.directory_handle(dirfd)?;
        if !parent_rights_base.contains(WasiRights::PATH_OPEN) {
            return Err(Errno::Notcapable);
        }
        let request = WasiPathOpen::from_preview1(oflags, rights_base, rights_inheriting, fdflags)?;
        let resolved = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_OPEN)?;
        self.open_resolved_path(
            resolved,
            request.options(),
            Some(request),
            parent_rights_inheriting,
        )
    }

    fn open_resolved_path(
        &mut self,
        resolved: NormalizedPath,
        options: WasiOpenOptions,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<WasiFd, Errno> {
        if let Ok(metadata) = self.namespace.metadata(&resolved)
            && metadata.file_type() == FileType::Directory
        {
            return self.open_directory_path(resolved, options, request, parent_rights_inheriting);
        }
        if request.is_some_and(WasiPathOpen::directory) {
            return Err(Errno::Notdir);
        }

        self.open_file_path(resolved, options, request, parent_rights_inheriting)
    }

    fn open_directory_path(
        &mut self,
        resolved: NormalizedPath,
        options: WasiOpenOptions,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<WasiFd, Errno> {
        if options.write || options.create || options.truncate {
            return Err(Errno::Isdir);
        }
        if options.append {
            return Err(Errno::Notcapable);
        }
        let (rights_base, rights_inheriting) = request.map_or_else(
            || {
                (
                    WasiRights::DIRECTORY_BASE.intersection(parent_rights_inheriting),
                    WasiRights::DIRECTORY_INHERITING.intersection(parent_rights_inheriting),
                )
            },
            |request| (request.rights_base(), request.rights_inheriting()),
        );
        let supported_directory_rights = if request.is_some() {
            WasiRights::DIRECTORY_INHERITING
        } else {
            WasiRights::DIRECTORY_BASE
        };
        if !supported_directory_rights.contains(rights_base) {
            return Err(Errno::Notcapable);
        }
        if let Some(request) = request
            && (!parent_rights_inheriting.contains(request.rights_base())
                || !parent_rights_inheriting.contains(request.rights_inheriting()))
        {
            return Err(Errno::Notcapable);
        }
        Ok(self.insert_handle(Handle::Directory {
            path: resolved,
            rights_base,
            rights_inheriting,
        }))
    }

    fn open_file_path(
        &mut self,
        resolved: NormalizedPath,
        options: WasiOpenOptions,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<WasiFd, Errno> {
        let request = FileOpenRequest::new(options, request, parent_rights_inheriting)?;
        let fd = self.next_file_fd()?;
        let file = self
            .namespace
            .open(&resolved, OpenOptions::from(options))
            .map_err(Errno::from)?;
        let rights_base = request.rights_base_for_opened_file(file.is_seekable())?;
        let file = WasiFile::new(
            file,
            resolved.as_str(),
            WasiFileAccess::new(options.read, options.write).with_append(options.append),
        );
        let fdflags = if options.append {
            WasiOpenOptions::FDFLAGS_APPEND
        } else {
            0
        };
        self.insert_file_handle(
            fd,
            OpenFileHandle {
                file,
                path: resolved,
                read: options.read,
                write: options.write,
                rights_base,
                fdflags,
            },
        )
    }

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
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.is_seekable_file().map_err(Errno::from)? {
                    return Err(Errno::Notcapable);
                }
                let from = file_seek_from(offset, whence)?;
                file.seek_file(from).map_err(Errno::from)
            }
            Handle::File {
                file, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_SEEK)
                    || !file.is_seekable_file().map_err(Errno::from)?
                {
                    return Err(Errno::Notcapable);
                }
                let from = file_seek_from(offset, whence)?;
                file.seek_file(from).map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Notcapable),
        }
    }

    /// Returns the current fd offset.
    pub fn fd_tell(&self, fd: WasiFd) -> Result<u64, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Stdio { file } => {
                if !file.is_seekable_file().map_err(Errno::from)? {
                    return Err(Errno::Notcapable);
                }
                file.tell_file().map_err(Errno::from)
            }
            Handle::File {
                file, rights_base, ..
            } => {
                if !rights_base.contains(WasiRights::FD_TELL)
                    || !file.is_seekable_file().map_err(Errno::from)?
                {
                    return Err(Errno::Notcapable);
                }
                file.tell_file().map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Notcapable),
        }
    }

    /// Returns stat data for a namespace path relative to `dirfd`.
    pub fn path_filestat_get(
        &self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<FileStat, Errno> {
        self.path_filestat_get_with_flags(dirfd, WasiLookupFlags::SYMLINK_FOLLOW, path)
    }

    /// Returns stat data for a namespace path using raw Preview 1 lookup flags.
    pub fn path_filestat_get_with_flags(
        &self,
        dirfd: WasiFd,
        flags: u32,
        path: impl AsRef<str>,
    ) -> Result<FileStat, Errno> {
        let lookup = WasiLookupFlags::from_preview1(flags)?;
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_FILESTAT_GET)?;
        self.stat_path_with_lookup(&path, lookup)
    }

    /// Sets access and modification times for a namespace path relative to `dirfd`.
    pub fn path_filestat_set_times(
        &self,
        dirfd: WasiFd,
        flags: u32,
        path: impl AsRef<str>,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        fstflags: u16,
    ) -> Result<(), Errno> {
        WasiLookupFlags::from_preview1(flags)?;
        let updates = WasiFilestatSetTimes::from_preview1(fstflags)?;
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_FILESTAT_SET_TIMES)?;
        self.set_path_times_with_updates(&path, accessed_time_ns, modified_time_ns, updates)
    }

    /// Creates a namespace directory at a path relative to `dirfd`.
    pub fn path_create_directory(&self, dirfd: WasiFd, path: impl AsRef<str>) -> Result<(), Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_CREATE_DIRECTORY)?;
        self.namespace.create_dir(&path).map_err(Errno::from)
    }

    /// Reads a symbolic link target at a namespace path relative to `dirfd`.
    pub fn path_readlink(&self, dirfd: WasiFd, path: impl AsRef<str>) -> Result<Vec<u8>, Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_READLINK)?;
        self.namespace.read_link(&path).map_err(Errno::from)
    }

    /// Creates a symbolic link at a namespace path relative to `dirfd`.
    pub fn path_symlink(
        &self,
        target: impl AsRef<[u8]>,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<(), Errno> {
        let target = wasi_symlink_target(target.as_ref())?;
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_SYMLINK)?;
        self.namespace.symlink(target, &path).map_err(Errno::from)
    }

    /// Removes a namespace directory at a path relative to `dirfd`.
    pub fn path_remove_directory(&self, dirfd: WasiFd, path: impl AsRef<str>) -> Result<(), Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_REMOVE_DIRECTORY)?;
        self.namespace.remove_dir(&path).map_err(Errno::from)
    }

    /// Renames a namespace path from one directory fd to another.
    pub fn path_rename(
        &self,
        old_fd: WasiFd,
        old_path: impl AsRef<str>,
        new_fd: WasiFd,
        new_path: impl AsRef<str>,
    ) -> Result<(), Errno> {
        let old_path =
            self.resolve_path(old_fd, old_path.as_ref(), WasiRights::PATH_RENAME_SOURCE)?;
        let new_path =
            self.resolve_path(new_fd, new_path.as_ref(), WasiRights::PATH_RENAME_TARGET)?;
        self.namespace
            .rename(&old_path, &new_path)
            .map_err(Errno::from)
    }

    /// Removes a namespace file at a path relative to `dirfd`.
    pub fn path_unlink_file(&self, dirfd: WasiFd, path: impl AsRef<str>) -> Result<(), Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_UNLINK_FILE)?;
        self.namespace.remove_file(&path).map_err(Errno::from)
    }

    /// Reads a directory at a namespace path relative to `dirfd`.
    pub fn path_read_dir(
        &self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<Vec<DirEntry>, Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_OPEN)?;
        self.namespace.read_dir(&path).map_err(Errno::from)
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

    fn insert_handle(&mut self, handle: Handle) -> WasiFd {
        let fd = WasiFd::new(self.next_fd);
        self.next_fd += 1;
        self.fds.insert(fd, handle);
        fd
    }

    fn insert_file_handle(&mut self, fd: WasiFd, handle: OpenFileHandle) -> Result<WasiFd, Errno> {
        if let Some(observer) = &self.fd_observer {
            observer.file_opened(fd, handle.file.clone(), &handle.path)?;
        }
        self.next_fd = self.next_fd.max(fd.get().saturating_add(1));
        self.fds.insert(
            fd,
            Handle::File {
                file: handle.file,
                path: handle.path,
                read: handle.read,
                write: handle.write,
                rights_base: handle.rights_base,
                fdflags: handle.fdflags,
            },
        );
        Ok(fd)
    }

    fn next_file_fd(&mut self) -> Result<WasiFd, Errno> {
        loop {
            let fd = WasiFd::new(self.next_fd);
            if self
                .fd_observer
                .as_ref()
                .is_none_or(|observer| observer.file_fd_available(fd))
            {
                return Ok(fd);
            }
            self.next_fd = self.next_fd.checked_add(1).ok_or(Errno::Inval)?;
        }
    }

    fn resolve_path(
        &self,
        dirfd: WasiFd,
        path: &str,
        required: WasiRights,
    ) -> Result<NormalizedPath, Errno> {
        let (base, rights_base, _) = self.directory_handle(dirfd)?;
        if !rights_base.contains(required) {
            return Err(Errno::Notcapable);
        }
        let path = wasi_path(path)?;
        if is_rooted_service_path(&path) {
            return Ok(path);
        }
        join_paths(base, &path).map_err(Errno::from)
    }

    fn directory_handle(
        &self,
        fd: WasiFd,
    ) -> Result<(&NormalizedPath, WasiRights, WasiRights), Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Preopen { source_path, .. } => Ok((
                source_path,
                WasiRights::DIRECTORY_BASE,
                WasiRights::DIRECTORY_INHERITING,
            )),
            Handle::Directory {
                path,
                rights_base,
                rights_inheriting,
            } => Ok((path, *rights_base, *rights_inheriting)),
            Handle::Stdio { .. } | Handle::File { .. } => Err(Errno::Notdir),
        }
    }

    fn stat_path(&self, path: &NormalizedPath) -> Result<FileStat, Errno> {
        self.stat_path_with_lookup(
            path,
            WasiLookupFlags::from_preview1(WasiLookupFlags::SYMLINK_FOLLOW)
                .expect("constant lookup flag is supported"),
        )
    }

    fn stat_path_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: WasiLookupFlags,
    ) -> Result<FileStat, Errno> {
        self.namespace
            .metadata_with_lookup(path, lookup.metadata_lookup())
            .map(FileStat::new)
            .map_err(Errno::from)
    }

    fn set_path_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        fstflags: u16,
    ) -> Result<(), Errno> {
        let updates = WasiFilestatSetTimes::from_preview1(fstflags)?;
        self.set_path_times_with_updates(path, accessed_time_ns, modified_time_ns, updates)
    }

    fn set_path_times_with_updates(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        updates: WasiFilestatSetTimes,
    ) -> Result<(), Errno> {
        if updates.is_empty() {
            return Ok(());
        }
        let current = self.stat_path(path)?;
        let accessed_time_ns = if updates.set_access_time() {
            accessed_time_ns
        } else if updates.set_access_time_to_now() {
            self.clock_time_ns
        } else {
            current.accessed_time_ns()
        };
        let modified_time_ns = if updates.set_modified_time() {
            modified_time_ns
        } else if updates.set_modified_time_to_now() {
            self.clock_time_ns
        } else {
            current.modified_time_ns()
        };
        self.namespace
            .set_times(path, accessed_time_ns, modified_time_ns)
            .map_err(Errno::from)
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

impl WasiCtx {
    fn next_dynamic_floor(&self) -> u32 {
        self.fds
            .keys()
            .take_while(|fd| fd.get() < self.next_fd)
            .filter(|fd| matches!(self.fds.get(fd), Some(Handle::Preopen { .. })))
            .map(|fd| fd.get())
            .max()
            .map_or(FIRST_PREOPEN_FD, |fd| fd + 1)
    }
}
