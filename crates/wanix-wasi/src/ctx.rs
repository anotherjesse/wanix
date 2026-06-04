use std::collections::BTreeMap;
use std::sync::Arc;

use wanix_fs::{FileSystem, FileType, NormalizedPath, OpenOptions};
use wanix_vfs::Namespace;

use crate::{
    Errno, WasiConfig, WasiFd, WasiFdObserver, WasiFile, WasiFileAccess, WasiOpenOptions,
    WasiPathOpen, WasiRights,
};

mod fd_ops;
mod handle;
mod open;
mod path;
mod path_ops;
mod seek;

use handle::{Handle, OpenFileHandle};
use open::FileOpenRequest;
use path::{is_rooted_service_path, join_paths, wasi_path};
pub use seek::WasiWhence;

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
