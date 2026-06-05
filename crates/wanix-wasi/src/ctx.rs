use std::{collections::BTreeMap, fmt, sync::Arc};

use wanix_fs::NormalizedPath;
use wanix_vfs::Namespace;

use crate::{Errno, WasiConfig, WasiFd, WasiFdObserver, WasiRights};

mod fd_ops;
mod handle;
mod init;
mod open;
mod path;
mod path_ops;
mod seek;

use handle::{Handle, OpenFileHandle};
use path::{is_rooted_service_path, join_paths, wasi_path};
pub use seek::WasiWhence;

const FIRST_PREOPEN_FD: u32 = 3;

/// Host context for Wanix-backed WASI filesystem operations.
pub struct WasiCtx {
    namespace: Namespace,
    fds: BTreeMap<WasiFd, Handle>,
    next_fd: u32,
    args: Vec<String>,
    env: Vec<String>,
    clock_time_ns: u64,
    fd_observer: Option<Arc<dyn WasiFdObserver>>,
}

impl fmt::Debug for WasiCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasiCtx")
            .field("namespace", &self.namespace)
            .field("fds", &self.fds)
            .field("next_fd", &self.next_fd)
            .field("arg_count", &self.args.len())
            .field("env_count", &self.env.len())
            .field("clock_time_ns", &self.clock_time_ns)
            .field("has_fd_observer", &self.fd_observer.is_some())
            .finish()
    }
}

impl WasiCtx {
    /// Creates a WASI context with fd 3 preopened at namespace root.
    #[must_use]
    pub fn new(config: WasiConfig) -> Self {
        Self::try_new(config).expect("WASI config preopens must be valid")
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
