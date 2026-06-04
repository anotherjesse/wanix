use std::fmt;
use std::sync::{Arc, Mutex};

mod types;

pub use types::{
    QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat,
    QuickJsWasiFileType, QuickJsWasiPrestat, QuickJsWasiWhence,
};

pub(crate) type QuickJsWasiHostHandle = Arc<Mutex<Box<dyn QuickJsWasiHost>>>;
const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;

/// Host-owned WASI Preview 1 hooks for QuickJS runtimes.
///
/// Implementations own process, descriptor, and namespace semantics. The
/// QuickJS engine only copies data between guest memory and this trait surface.
pub trait QuickJsWasiHost: Send {
    /// Returns host-owned live resources that make VM snapshotting unsafe.
    ///
    /// Snapshot bytes contain QuickJS WebAssembly memory only. Implementations
    /// should report any open descriptor, callback, or host resource state that
    /// cannot be safely serialized into that VM image.
    fn snapshot_blockers(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        Ok(Vec::new())
    }

    /// Returns process arguments in WASI argv order.
    fn args(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        Ok(Vec::new())
    }

    /// Returns process environment strings in `KEY=value` form.
    fn env(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        Ok(Vec::new())
    }

    /// Requests process exit with a WASI exit code.
    fn proc_exit(&mut self, _code: u32) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Returns metadata for a preopened directory fd.
    fn fd_prestat_get(&mut self, fd: u32) -> Result<QuickJsWasiPrestat, QuickJsWasiErrno>;

    /// Opens `path` relative to `dirfd` using raw Preview 1 rights and flags.
    #[allow(clippy::too_many_arguments)]
    fn path_open(
        &mut self,
        dirfd: u32,
        dirflags: u32,
        path: &[u8],
        oflags: u16,
        rights_base: u64,
        rights_inheriting: u64,
        fdflags: u16,
    ) -> Result<u32, QuickJsWasiErrno>;

    /// Reads from an open fd into `buf`.
    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> Result<usize, QuickJsWasiErrno>;

    /// Returns whether a read subscription should report this fd as ready now.
    ///
    /// The default preserves the earlier live-host behavior: if the fd exists
    /// and has read rights, it is considered ready. Hosts with device queues can
    /// override this to avoid firing read handlers while no input is available.
    fn fd_read_ready(&mut self, fd: u32) -> Result<bool, QuickJsWasiErrno> {
        let stat = self.fd_fdstat_get(fd)?;
        if stat.rights_base() & RIGHT_FD_READ == 0 {
            Err(QuickJsWasiErrno::Notcapable)
        } else {
            Ok(true)
        }
    }

    /// Returns directory entries for an open directory fd.
    fn fd_readdir(&mut self, fd: u32) -> Result<Vec<QuickJsWasiDirEntry>, QuickJsWasiErrno>;

    /// Writes bytes from `buf` to an open fd.
    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> Result<usize, QuickJsWasiErrno>;

    /// Returns whether a write subscription should report this fd as ready now.
    ///
    /// The default preserves the earlier live-host behavior: writable fds are
    /// ready immediately.
    fn fd_write_ready(&mut self, fd: u32) -> Result<bool, QuickJsWasiErrno> {
        let stat = self.fd_fdstat_get(fd)?;
        if stat.rights_base() & RIGHT_FD_WRITE == 0 {
            Err(QuickJsWasiErrno::Notcapable)
        } else {
            Ok(true)
        }
    }

    /// Seeks an open fd and returns the resulting offset.
    fn fd_seek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: QuickJsWasiWhence,
    ) -> Result<u64, QuickJsWasiErrno>;

    /// Returns the current offset for an open fd.
    fn fd_tell(&mut self, _fd: u32) -> Result<u64, QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Closes an open fd.
    fn fd_close(&mut self, fd: u32) -> Result<(), QuickJsWasiErrno>;

    /// Returns fdstat metadata for an open fd.
    fn fd_fdstat_get(&mut self, fd: u32) -> Result<QuickJsWasiFdStat, QuickJsWasiErrno>;

    /// Updates mutable Preview 1 fdflags for an open fd.
    fn fd_fdstat_set_flags(&mut self, _fd: u32, _fdflags: u16) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Returns filestat metadata for an open fd.
    fn fd_filestat_get(&mut self, fd: u32) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno>;

    /// Sets access and modification times for an open fd.
    fn fd_filestat_set_times(
        &mut self,
        _fd: u32,
        _atim: u64,
        _mtim: u64,
        _fstflags: u16,
    ) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Sets the size for an open fd.
    fn fd_filestat_set_size(&mut self, _fd: u32, _size: u64) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Returns filestat metadata for `path` relative to `dirfd`.
    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        flags: u32,
        path: &[u8],
    ) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno>;

    /// Sets access and modification times for `path` relative to `dirfd`.
    fn path_filestat_set_times(
        &mut self,
        _dirfd: u32,
        _flags: u32,
        _path: &[u8],
        _atim: u64,
        _mtim: u64,
        _fstflags: u16,
    ) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Creates a directory at `path` relative to `dirfd`.
    fn path_create_directory(&mut self, _dirfd: u32, _path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Reads a symbolic link target at `path` relative to `dirfd`.
    fn path_readlink(&mut self, _dirfd: u32, _path: &[u8]) -> Result<Vec<u8>, QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Creates a symbolic link at `path` relative to `dirfd`.
    fn path_symlink(
        &mut self,
        _target: &[u8],
        _dirfd: u32,
        _path: &[u8],
    ) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Removes a directory at `path` relative to `dirfd`.
    fn path_remove_directory(&mut self, _dirfd: u32, _path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Renames `old_path` relative to `old_fd` to `new_path` relative to `new_fd`.
    fn path_rename(
        &mut self,
        _old_fd: u32,
        _old_path: &[u8],
        _new_fd: u32,
        _new_path: &[u8],
    ) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }

    /// Removes a non-directory file at `path` relative to `dirfd`.
    fn path_unlink_file(&mut self, _dirfd: u32, _path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        Err(QuickJsWasiErrno::Nosys)
    }
}

impl fmt::Debug for dyn QuickJsWasiHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dyn QuickJsWasiHost")
    }
}
