use crate::Errno;

use super::WasiRights;

/// Open options for Wanix-backed WASI path opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiOpenOptions {
    /// Request read rights.
    pub read: bool,
    /// Request write rights.
    pub write: bool,
    /// Create missing file.
    pub create: bool,
    /// Truncate existing file.
    pub truncate: bool,
    /// Seek to the current end of file before every write.
    pub append: bool,
}

impl WasiOpenOptions {
    /// Preview 1 `oflags` bit for create.
    pub const OFLAGS_CREATE: u16 = 1 << 0;
    /// Preview 1 `oflags` bit requiring a directory.
    pub const OFLAGS_DIRECTORY: u16 = 1 << 1;
    /// Preview 1 `oflags` bit for exclusive create.
    pub const OFLAGS_EXCLUSIVE: u16 = 1 << 2;
    /// Preview 1 `oflags` bit for truncate.
    pub const OFLAGS_TRUNCATE: u16 = 1 << 3;
    /// Preview 1 `fdflags` bit for append mode.
    pub const FDFLAGS_APPEND: u16 = 1 << 0;
    /// Preview 1 `fdflags` bit for data-sync writes.
    pub const FDFLAGS_DSYNC: u16 = 1 << 1;
    /// Preview 1 `fdflags` bit for non-blocking mode.
    pub const FDFLAGS_NONBLOCK: u16 = 1 << 2;
    /// Preview 1 `fdflags` bit for read-sync behavior.
    pub const FDFLAGS_RSYNC: u16 = 1 << 3;
    /// Preview 1 `fdflags` bit for sync writes.
    pub const FDFLAGS_SYNC: u16 = 1 << 4;

    const SUPPORTED_OFLAGS: u16 = Self::OFLAGS_CREATE | Self::OFLAGS_TRUNCATE;
    const SUPPORTED_FDFLAGS: u16 = Self::FDFLAGS_APPEND;

    /// Read-only file open.
    #[must_use]
    pub fn read() -> Self {
        Self {
            read: true,
            write: false,
            create: false,
            truncate: false,
            append: false,
        }
    }

    /// Read-write file open.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            create: false,
            truncate: false,
            append: false,
        }
    }

    /// Write/create file open.
    #[must_use]
    pub fn create_write() -> Self {
        Self {
            read: false,
            write: true,
            create: true,
            truncate: false,
            append: false,
        }
    }

    /// Converts Preview 1 `path_open` flags and base rights into open options.
    ///
    /// Directory-only opens, exclusive creation, and non-append fdflags are
    /// rejected because generic file open options cannot express directory
    /// handles. Use [`WasiPathOpen::from_preview1`] for raw Preview 1 requests.
    pub fn from_preview1(
        oflags: u16,
        rights_base: WasiRights,
        fdflags: u16,
    ) -> Result<Self, Errno> {
        Self::validate_preview1_fdflags(fdflags)?;
        if oflags & !Self::SUPPORTED_OFLAGS != 0 {
            return Err(Errno::Notcapable);
        }
        if rights_base.bits() & !WasiRights::OPEN_FILE_BASE.bits() != 0 {
            return Err(Errno::Notcapable);
        }
        let read = rights_base.contains(WasiRights::FD_READ);
        let write = rights_base.contains(WasiRights::FD_WRITE);
        let create = oflags & Self::OFLAGS_CREATE != 0;
        let truncate = oflags & Self::OFLAGS_TRUNCATE != 0;
        let append = fdflags & Self::FDFLAGS_APPEND != 0;
        if (create || truncate || append) && !write {
            return Err(Errno::Notcapable);
        }
        Ok(Self {
            read,
            write,
            create,
            truncate,
            append,
        })
    }

    pub(crate) fn validate_preview1_fdflags(fdflags: u16) -> Result<(), Errno> {
        if fdflags & !Self::SUPPORTED_FDFLAGS != 0 {
            return Err(Errno::Notcapable);
        }
        Ok(())
    }
}

/// Preview 1 path-open request with requested fd rights preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiPathOpen {
    options: WasiOpenOptions,
    rights_base: WasiRights,
    rights_inheriting: WasiRights,
    directory: bool,
}

impl WasiPathOpen {
    /// Converts raw Preview 1 `path_open` flags and rights into a request.
    ///
    /// Directory opens are preserved as request metadata because `WasiOpenOptions`
    /// represents file open behavior. Non-blocking directory fdflags are accepted
    /// for QuickJS libc compatibility and currently do not affect synchronous
    /// Wanix namespace operations.
    pub fn from_preview1(
        oflags: u16,
        rights_base: WasiRights,
        rights_inheriting: WasiRights,
        fdflags: u16,
    ) -> Result<Self, Errno> {
        if fdflags & !(WasiOpenOptions::FDFLAGS_APPEND | WasiOpenOptions::FDFLAGS_NONBLOCK) != 0 {
            return Err(Errno::Notcapable);
        }
        if oflags & !(WasiOpenOptions::SUPPORTED_OFLAGS | WasiOpenOptions::OFLAGS_DIRECTORY) != 0 {
            return Err(Errno::Notcapable);
        }
        // libc/std guests request rights wholesale (Rust std 1.93 asks for
        // fd_sync/fd_advise/fd_fdstat_set_flags/path_link/poll_fd_readwrite on
        // every open). Preview 1 rights are advisory and deprecated, so clamp
        // the request to the modeled subset instead of refusing the open; the
        // granted fd rights are always the modeled intersection, and the
        // unimplemented syscalls behind the dropped bits still refuse honestly.
        let rights_base = rights_base.intersection(WasiRights::DIRECTORY_INHERITING);
        let rights_inheriting = rights_inheriting.intersection(WasiRights::DIRECTORY_INHERITING);
        let create = oflags & WasiOpenOptions::OFLAGS_CREATE != 0;
        let directory = oflags & WasiOpenOptions::OFLAGS_DIRECTORY != 0;
        let truncate = oflags & WasiOpenOptions::OFLAGS_TRUNCATE != 0;
        let append = fdflags & WasiOpenOptions::FDFLAGS_APPEND != 0;
        if directory && (create || truncate || append) {
            return Err(Errno::Notcapable);
        }
        if (create || truncate || append) && !rights_base.contains(WasiRights::FD_WRITE) {
            return Err(Errno::Notcapable);
        }
        let options = WasiOpenOptions {
            read: rights_base.contains(WasiRights::FD_READ),
            write: rights_base.contains(WasiRights::FD_WRITE),
            create,
            truncate,
            append,
        };
        Ok(Self {
            options,
            rights_base,
            rights_inheriting,
            directory,
        })
    }

    /// Returns Wanix open options derived from requested rights and flags.
    #[must_use]
    pub const fn options(self) -> WasiOpenOptions {
        self.options
    }

    /// Returns rights requested for the opened fd.
    #[must_use]
    pub const fn rights_base(self) -> WasiRights {
        self.rights_base
    }

    /// Returns rights requested for child fds opened from the opened fd.
    #[must_use]
    pub const fn rights_inheriting(self) -> WasiRights {
        self.rights_inheriting
    }

    /// Returns whether Preview 1 required the path to name a directory.
    #[must_use]
    pub const fn directory(self) -> bool {
        self.directory
    }

    pub(crate) const fn file_rights_base(self) -> WasiRights {
        self.rights_base.intersection(WasiRights::OPEN_FILE_BASE)
    }
}

impl From<WasiOpenOptions> for wanix_fs::OpenOptions {
    fn from(options: WasiOpenOptions) -> Self {
        Self {
            read: options.read,
            write: options.write,
            create: options.create,
            truncate: options.truncate,
        }
    }
}
