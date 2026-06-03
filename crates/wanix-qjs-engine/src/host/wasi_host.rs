use std::fmt;
use std::sync::{Arc, Mutex};

pub(crate) type QuickJsWasiHostHandle = Arc<Mutex<Box<dyn QuickJsWasiHost>>>;

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

    /// Returns directory entries for an open directory fd.
    fn fd_readdir(&mut self, fd: u32) -> Result<Vec<QuickJsWasiDirEntry>, QuickJsWasiErrno>;

    /// Writes bytes from `buf` to an open fd.
    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> Result<usize, QuickJsWasiErrno>;

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

    /// Returns filestat metadata for `path` relative to `dirfd`.
    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        flags: u32,
        path: &[u8],
    ) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno>;

    /// Creates a directory at `path` relative to `dirfd`.
    fn path_create_directory(&mut self, _dirfd: u32, _path: &[u8]) -> Result<(), QuickJsWasiErrno> {
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

/// WASI Preview 1 errno values used by the QuickJS engine host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiErrno {
    /// Bad file descriptor.
    Badf,
    /// Invalid input.
    Inval,
    /// Path or name is too long.
    Nametoolong,
    /// File or directory missing.
    Noent,
    /// File exists.
    Exist,
    /// Generic I/O error.
    Io,
    /// Path names a directory where a file was expected.
    Isdir,
    /// Path component was not a directory.
    Notdir,
    /// Directory is not empty.
    Notempty,
    /// Operation not supported.
    Nosys,
    /// Capability rights are insufficient.
    Notcapable,
}

impl QuickJsWasiErrno {
    pub(crate) const fn preview1_result(self) -> i32 {
        match self {
            Self::Badf => 8,
            Self::Exist => 20,
            Self::Inval => 28,
            Self::Io => 29,
            Self::Isdir => 31,
            Self::Nametoolong => 37,
            Self::Noent => 44,
            Self::Nosys => 52,
            Self::Notdir => 54,
            Self::Notempty => 55,
            Self::Notcapable => 76,
        }
    }
}

/// WASI Preview 1 file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiFileType {
    /// Unknown file type.
    Unknown,
    /// Character device.
    CharacterDevice,
    /// Directory.
    Directory,
    /// Regular file.
    RegularFile,
    /// Symbolic link.
    SymbolicLink,
}

impl QuickJsWasiFileType {
    /// Creates a file type from a Preview 1 numeric code.
    #[must_use]
    pub const fn from_preview1_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            2 => Some(Self::CharacterDevice),
            3 => Some(Self::Directory),
            4 => Some(Self::RegularFile),
            7 => Some(Self::SymbolicLink),
            _ => None,
        }
    }

    pub(crate) const fn preview1_code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::CharacterDevice => 2,
            Self::Directory => 3,
            Self::RegularFile => 4,
            Self::SymbolicLink => 7,
        }
    }
}

/// WASI Preview 1 seek origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiWhence {
    /// Seek relative to the start.
    Set,
    /// Seek relative to the current offset.
    Cur,
    /// Seek relative to the end.
    End,
}

impl QuickJsWasiWhence {
    pub(crate) const fn from_preview1(code: i32) -> Result<Self, QuickJsWasiErrno> {
        match code {
            0 => Ok(Self::Set),
            1 => Ok(Self::Cur),
            2 => Ok(Self::End),
            _ => Err(QuickJsWasiErrno::Inval),
        }
    }
}

/// WASI Preview 1 prestat metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickJsWasiPrestat {
    dir_name: String,
}

impl QuickJsWasiPrestat {
    /// Creates prestat metadata for a preopened directory name.
    #[must_use]
    pub fn new(dir_name: impl Into<String>) -> Self {
        Self {
            dir_name: dir_name.into(),
        }
    }

    pub(crate) fn dir_name(&self) -> &str {
        &self.dir_name
    }
}

/// WASI Preview 1 directory entry metadata returned by a live host provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickJsWasiDirEntry {
    name: String,
    file_type: QuickJsWasiFileType,
}

impl QuickJsWasiDirEntry {
    /// Creates directory entry metadata.
    #[must_use]
    pub fn new(name: impl Into<String>, file_type: QuickJsWasiFileType) -> Self {
        Self {
            name: name.into(),
            file_type,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn file_type(&self) -> QuickJsWasiFileType {
        self.file_type
    }
}

/// WASI Preview 1 fdstat metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickJsWasiFdStat {
    file_type: QuickJsWasiFileType,
    fdflags: u16,
    rights_base: u64,
    rights_inheriting: u64,
}

impl QuickJsWasiFdStat {
    /// Creates fdstat metadata.
    #[must_use]
    pub const fn new(
        file_type: QuickJsWasiFileType,
        rights_base: u64,
        rights_inheriting: u64,
    ) -> Self {
        Self::new_with_fdflags(file_type, 0, rights_base, rights_inheriting)
    }

    /// Creates fdstat metadata with Preview 1 fdflags.
    #[must_use]
    pub const fn new_with_fdflags(
        file_type: QuickJsWasiFileType,
        fdflags: u16,
        rights_base: u64,
        rights_inheriting: u64,
    ) -> Self {
        Self {
            file_type,
            fdflags,
            rights_base,
            rights_inheriting,
        }
    }

    pub(crate) const fn file_type(self) -> QuickJsWasiFileType {
        self.file_type
    }

    pub(crate) const fn fdflags(self) -> u16 {
        self.fdflags
    }

    pub(crate) const fn rights_base(self) -> u64 {
        self.rights_base
    }

    pub(crate) const fn rights_inheriting(self) -> u64 {
        self.rights_inheriting
    }
}

/// WASI Preview 1 filestat metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickJsWasiFileStat {
    file_type: QuickJsWasiFileType,
    size: u64,
}

impl QuickJsWasiFileStat {
    /// Creates filestat metadata.
    #[must_use]
    pub const fn new(file_type: QuickJsWasiFileType, size: u64) -> Self {
        Self { file_type, size }
    }

    pub(crate) const fn file_type(self) -> QuickJsWasiFileType {
        self.file_type
    }

    pub(crate) const fn size(self) -> u64 {
        self.size
    }
}
