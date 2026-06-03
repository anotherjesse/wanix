use std::collections::BTreeMap;
use std::fmt;

use wanix_fs::{DirEntry, File, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};
use wanix_vfs::Namespace;

use crate::{Errno, FileStat, WasiConfig, WasiFd, WasiOpenOptions};

const FIRST_PREOPEN_FD: u32 = 3;
const MAX_WASI_PATH_BYTES: usize = 4096;

/// Host context for Wanix-backed WASI filesystem operations.
#[derive(Debug)]
pub struct WasiCtx {
    namespace: Namespace,
    fds: BTreeMap<WasiFd, Handle>,
    next_fd: u32,
}

enum Handle {
    Preopen {
        path: NormalizedPath,
    },
    Directory {
        path: NormalizedPath,
    },
    File {
        file: Box<dyn File>,
        path: NormalizedPath,
        read: bool,
        write: bool,
    },
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preopen { path } => f.debug_struct("Preopen").field("path", path).finish(),
            Self::Directory { path } => f.debug_struct("Directory").field("path", path).finish(),
            Self::File {
                path, read, write, ..
            } => f
                .debug_struct("File")
                .field("path", path)
                .field("read", read)
                .field("write", write)
                .finish(),
        }
    }
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
        for (index, preopen) in config.preopens().iter().enumerate() {
            let path = preopen.guest_path().clone();
            let metadata = config.namespace().metadata(&path).map_err(Errno::from)?;
            if metadata.file_type() != FileType::Directory {
                return Err(Errno::Notdir);
            }
            let fd =
                WasiFd::new(FIRST_PREOPEN_FD + u32::try_from(index).map_err(|_| Errno::Inval)?);
            fds.insert(fd, Handle::Preopen { path });
        }
        let next_fd = FIRST_PREOPEN_FD + u32::try_from(fds.len()).map_err(|_| Errno::Inval)?;
        Ok(Self {
            namespace: config.namespace().clone(),
            fds,
            next_fd,
        })
    }

    /// Returns the namespace backing this WASI context.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Opens a namespace path relative to `dirfd`.
    pub fn path_open(
        &mut self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
        options: WasiOpenOptions,
    ) -> Result<WasiFd, Errno> {
        let resolved = self.resolve_path(dirfd, path.as_ref())?;
        if let Ok(metadata) = self.namespace.metadata(&resolved)
            && metadata.file_type() == FileType::Directory
        {
            if options.write || options.create || options.truncate {
                return Err(Errno::Isdir);
            }
            return Ok(self.insert_handle(Handle::Directory { path: resolved }));
        }

        let file = self
            .namespace
            .open(&resolved, OpenOptions::from(options))
            .map_err(Errno::from)?;
        Ok(self.insert_handle(Handle::File {
            file,
            path: resolved,
            read: options.read,
            write: options.write,
        }))
    }

    /// Reads bytes from an open fd.
    pub fn fd_read(&mut self, fd: WasiFd, buf: &mut [u8]) -> Result<usize, Errno> {
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::File { file, read, .. } => {
                if !*read {
                    return Err(Errno::Notcapable);
                }
                file.read(buf).map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Writes bytes to an open fd.
    pub fn fd_write(&mut self, fd: WasiFd, buf: &[u8]) -> Result<usize, Errno> {
        match self.fds.get_mut(&fd).ok_or(Errno::Badf)? {
            Handle::File { file, write, .. } => {
                if !*write {
                    return Err(Errno::Notcapable);
                }
                file.write(buf).map_err(Errno::from)
            }
            Handle::Preopen { .. } | Handle::Directory { .. } => Err(Errno::Isdir),
        }
    }

    /// Closes a dynamic fd.
    pub fn fd_close(&mut self, fd: WasiFd) -> Result<(), Errno> {
        if fd.get() < self.next_dynamic_floor() {
            return Err(Errno::Badf);
        }
        self.fds.remove(&fd).map(|_| ()).ok_or(Errno::Badf)
    }

    /// Returns stat data for an open fd.
    pub fn fd_filestat_get(&self, fd: WasiFd) -> Result<FileStat, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Preopen { path } | Handle::Directory { path } => self.stat_path(path),
            Handle::File { file, .. } => file.metadata().map(FileStat::new).map_err(Errno::from),
        }
    }

    /// Returns stat data for a namespace path relative to `dirfd`.
    pub fn path_filestat_get(
        &self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<FileStat, Errno> {
        let path = self.resolve_path(dirfd, path.as_ref())?;
        self.stat_path(&path)
    }

    /// Reads a directory at a namespace path relative to `dirfd`.
    pub fn path_read_dir(
        &self,
        dirfd: WasiFd,
        path: impl AsRef<str>,
    ) -> Result<Vec<DirEntry>, Errno> {
        let path = self.resolve_path(dirfd, path.as_ref())?;
        self.namespace.read_dir(&path).map_err(Errno::from)
    }

    /// Reads a directory from an open preopen or directory fd.
    pub fn fd_read_dir(&self, fd: WasiFd) -> Result<Vec<DirEntry>, Errno> {
        match self.fds.get(&fd).ok_or(Errno::Badf)? {
            Handle::Preopen { path } | Handle::Directory { path } => {
                self.namespace.read_dir(path).map_err(Errno::from)
            }
            Handle::File { .. } => Err(Errno::Notdir),
        }
    }

    fn insert_handle(&mut self, handle: Handle) -> WasiFd {
        let fd = WasiFd::new(self.next_fd);
        self.next_fd += 1;
        self.fds.insert(fd, handle);
        fd
    }

    fn resolve_path(&self, dirfd: WasiFd, path: &str) -> Result<NormalizedPath, Errno> {
        let base = match self.fds.get(&dirfd).ok_or(Errno::Badf)? {
            Handle::Preopen { path } | Handle::Directory { path } => path,
            Handle::File { .. } => return Err(Errno::Notdir),
        };
        let path = wasi_path(path)?;
        join_paths(base, &path).map_err(Errno::from)
    }

    fn stat_path(&self, path: &NormalizedPath) -> Result<FileStat, Errno> {
        self.namespace
            .metadata(path)
            .map(FileStat::new)
            .map_err(Errno::from)
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

fn wasi_path(path: &str) -> Result<NormalizedPath, Errno> {
    if path.len() > MAX_WASI_PATH_BYTES {
        return Err(Errno::Nametoolong);
    }
    if path == "." {
        return NormalizedPath::new(path).map_err(Errno::from);
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|component| component == "." || component == "..")
    {
        return Err(Errno::Notcapable);
    }
    NormalizedPath::new(path).map_err(Errno::from)
}

fn join_paths(base: &NormalizedPath, path: &NormalizedPath) -> Result<NormalizedPath, FsError> {
    if path.as_str() == "." {
        return Ok(base.clone());
    }
    if base.as_str() == "." {
        return Ok(path.clone());
    }
    NormalizedPath::new(format!("{base}/{path}"))
}
