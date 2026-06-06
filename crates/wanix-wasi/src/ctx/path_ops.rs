use wanix_fs::{DirEntry, FileSystem, NormalizedPath};

use crate::{Errno, FileStat, WasiCtx, WasiFd, WasiFilestatSetTimes, WasiLookupFlags, WasiRights};

use super::path::wasi_symlink_target;

impl WasiCtx {
    /// Returns stat data for a namespace path relative to `dirfd`.
    pub fn path_filestat_get(
        &self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<FileStat, Errno> {
        let path = self.resolve_path(dirfd, path.as_ref(), WasiRights::PATH_FILESTAT_GET)?;
        self.stat_path_with_lookup(&path, WasiLookupFlags::FOLLOW_SYMLINKS)
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

    pub(super) fn stat_path(&self, path: &NormalizedPath) -> Result<FileStat, Errno> {
        self.stat_path_with_lookup(path, WasiLookupFlags::FOLLOW_SYMLINKS)
    }

    pub(super) fn stat_path_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: WasiLookupFlags,
    ) -> Result<FileStat, Errno> {
        self.namespace
            .metadata_with_lookup(path, lookup.metadata_lookup())
            .map(FileStat::new)
            .map_err(Errno::from)
    }

    pub(super) fn set_path_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        fstflags: u16,
    ) -> Result<(), Errno> {
        let updates = WasiFilestatSetTimes::from_preview1(fstflags)?;
        self.set_path_times_with_updates(path, accessed_time_ns, modified_time_ns, updates)
    }

    pub(super) fn set_path_times_with_updates(
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
