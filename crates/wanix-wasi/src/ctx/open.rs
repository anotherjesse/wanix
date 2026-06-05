use wanix_fs::{FileSystem, FileType, NormalizedPath, OpenOptions};

use crate::{
    Errno, WasiCtx, WasiFd, WasiFile, WasiFileAccess, WasiOpenOptions, WasiPathOpen, WasiRights,
};

use super::handle::{Handle, OpenFileHandle};

mod directory;
mod file;

use directory::{DirectoryOpenRights, reject_directory_write_options};
use file::{FileOpenRequest, append_fdflags};

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
