use wanix_fs::{FileSystem, FileType, NormalizedPath, OpenOptions};

use crate::{
    Errno, WasiCtx, WasiFd, WasiFile, WasiFileAccess, WasiOpenOptions, WasiPathOpen, WasiRights,
};

use super::handle::{Handle, OpenFileHandle};

pub(super) struct FileOpenRequest {
    options: WasiOpenOptions,
    request: Option<WasiPathOpen>,
    parent_rights_inheriting: WasiRights,
}

impl FileOpenRequest {
    pub(super) fn new(
        options: WasiOpenOptions,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<Self, Errno> {
        let request = Self {
            options,
            request,
            parent_rights_inheriting,
        };
        request.validate_requested_rights()?;
        Ok(request)
    }

    pub(super) fn rights_base_for_opened_file(&self, seekable: bool) -> Result<WasiRights, Errno> {
        let supported_rights = open_file_rights(self.options.read, self.options.write, seekable);
        let rights_base = self.request.map_or_else(
            || supported_rights.intersection(self.parent_rights_inheriting),
            |request| request.file_rights_base().intersection(supported_rights),
        );
        self.validate_required_io_rights(rights_base)?;
        Ok(rights_base)
    }

    fn validate_requested_rights(&self) -> Result<(), Errno> {
        let requested_file_rights = open_file_rights(self.options.read, self.options.write, true);
        if let Some(request) = self.request {
            let requested_base = request.file_rights_base();
            if !requested_file_rights.contains(requested_base)
                || !self.parent_rights_inheriting.contains(requested_base)
            {
                return Err(Errno::Notcapable);
            }
            return Ok(());
        }

        let default_file_rights = requested_file_rights.intersection(self.parent_rights_inheriting);
        self.validate_required_io_rights(default_file_rights)
    }

    fn validate_required_io_rights(&self, rights: WasiRights) -> Result<(), Errno> {
        if self.options.read && !rights.contains(WasiRights::FD_READ) {
            return Err(Errno::Notcapable);
        }
        if self.options.write && !rights.contains(WasiRights::FD_WRITE) {
            return Err(Errno::Notcapable);
        }
        Ok(())
    }
}

impl WasiCtx {
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
        reject_directory_write_options(options)?;
        let rights = DirectoryOpenRights::new(request, parent_rights_inheriting)?;
        Ok(self.insert_handle(Handle::Directory {
            path: resolved,
            rights_base: rights.base,
            rights_inheriting: rights.inheriting,
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
        let fdflags = append_fdflags(options);
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
}

struct DirectoryOpenRights {
    base: WasiRights,
    inheriting: WasiRights,
}

impl DirectoryOpenRights {
    fn new(
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<Self, Errno> {
        let rights = request.map_or_else(
            || Self::default_from_parent(parent_rights_inheriting),
            Self::requested,
        );
        rights.validate_supported(request)?;
        rights.validate_parent_rights(request, parent_rights_inheriting)?;
        Ok(rights)
    }

    fn default_from_parent(parent_rights_inheriting: WasiRights) -> Self {
        Self {
            base: WasiRights::DIRECTORY_BASE.intersection(parent_rights_inheriting),
            inheriting: WasiRights::DIRECTORY_INHERITING.intersection(parent_rights_inheriting),
        }
    }

    const fn requested(request: WasiPathOpen) -> Self {
        Self {
            base: request.rights_base(),
            inheriting: request.rights_inheriting(),
        }
    }

    fn validate_supported(&self, request: Option<WasiPathOpen>) -> Result<(), Errno> {
        let supported = if request.is_some() {
            WasiRights::DIRECTORY_INHERITING
        } else {
            WasiRights::DIRECTORY_BASE
        };
        if supported.contains(self.base) {
            return Ok(());
        }
        Err(Errno::Notcapable)
    }

    fn validate_parent_rights(
        &self,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<(), Errno> {
        if request.is_none()
            || (parent_rights_inheriting.contains(self.base)
                && parent_rights_inheriting.contains(self.inheriting))
        {
            return Ok(());
        }
        Err(Errno::Notcapable)
    }
}

fn open_file_rights(read: bool, write: bool, seekable: bool) -> WasiRights {
    let mut rights = WasiRights::FD_FILESTAT_GET | WasiRights::FD_FILESTAT_SET_TIMES;
    if read {
        rights |= WasiRights::FD_READ;
    }
    if write {
        rights |= WasiRights::FD_WRITE | WasiRights::FD_FILESTAT_SET_SIZE;
    }
    if seekable {
        rights |= WasiRights::FD_SEEK | WasiRights::FD_TELL;
    }
    rights
}

fn reject_directory_write_options(options: WasiOpenOptions) -> Result<(), Errno> {
    if options.write || options.create || options.truncate {
        return Err(Errno::Isdir);
    }
    if options.append {
        return Err(Errno::Notcapable);
    }
    Ok(())
}

fn append_fdflags(options: WasiOpenOptions) -> u16 {
    if options.append {
        WasiOpenOptions::FDFLAGS_APPEND
    } else {
        0
    }
}
