use std::ops::{BitOr, BitOrAssign};

use wanix_fs::{FileType, NormalizedPath};

use crate::Errno;

/// WASI file descriptor identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WasiFd(u32);

impl WasiFd {
    /// Standard input.
    pub const STDIN: Self = Self(0);
    /// Standard output.
    pub const STDOUT: Self = Self(1);
    /// Standard error.
    pub const STDERR: Self = Self(2);
    /// First root preopen fd.
    pub const ROOT: Self = Self(3);

    /// Creates a WASI fd.
    #[must_use]
    pub fn new(fd: u32) -> Self {
        Self(fd)
    }

    /// Returns the numeric fd.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// WASI Preview 1 file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasiFileType {
    /// Unknown file type.
    Unknown,
    /// Character device such as stdin/stdout/stderr.
    CharacterDevice,
    /// Directory.
    Directory,
    /// Regular byte file.
    RegularFile,
    /// Symbolic link.
    SymbolicLink,
}

impl WasiFileType {
    /// Returns the WASI Preview 1 numeric filetype code.
    #[must_use]
    pub const fn preview1_code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::CharacterDevice => 2,
            Self::Directory => 3,
            Self::RegularFile => 4,
            Self::SymbolicLink => 7,
        }
    }

    pub(crate) const fn from_wanix(file_type: FileType) -> Self {
        match file_type {
            FileType::File => Self::RegularFile,
            FileType::Directory => Self::Directory,
            FileType::Symlink => Self::SymbolicLink,
        }
    }
}

/// WASI Preview 1 rights bitset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WasiRights(u64);

impl WasiRights {
    /// Empty rights set.
    pub const NONE: Self = Self(0);
    /// Read from an fd.
    pub const FD_READ: Self = Self(1 << 1);
    /// Seek on an fd.
    pub const FD_SEEK: Self = Self(1 << 2);
    /// Tell an fd offset.
    pub const FD_TELL: Self = Self(1 << 5);
    /// Write to an fd.
    pub const FD_WRITE: Self = Self(1 << 6);
    /// Create a directory at a path relative to this fd.
    pub const PATH_CREATE_DIRECTORY: Self = Self(1 << 9);
    /// Create a file at a path relative to this fd.
    pub const PATH_CREATE_FILE: Self = Self(1 << 10);
    /// Open a path relative to this fd.
    pub const PATH_OPEN: Self = Self(1 << 13);
    /// Read directory entries from an fd.
    pub const FD_READDIR: Self = Self(1 << 14);
    /// Rename a source path relative to this fd.
    pub const PATH_RENAME_SOURCE: Self = Self(1 << 16);
    /// Rename a target path relative to this fd.
    pub const PATH_RENAME_TARGET: Self = Self(1 << 17);
    /// Stat a path relative to this fd.
    pub const PATH_FILESTAT_GET: Self = Self(1 << 18);
    /// Set the size of a path relative to this fd.
    pub const PATH_FILESTAT_SET_SIZE: Self = Self(1 << 19);
    /// Set access or modification times on a path relative to this fd.
    pub const PATH_FILESTAT_SET_TIMES: Self = Self(1 << 20);
    /// Stat this fd.
    pub const FD_FILESTAT_GET: Self = Self(1 << 21);
    /// Set access or modification times on this fd.
    pub const FD_FILESTAT_SET_TIMES: Self = Self(1 << 23);
    /// Remove a directory at a path relative to this fd.
    pub const PATH_REMOVE_DIRECTORY: Self = Self(1 << 25);
    /// Remove a non-directory file at a path relative to this fd.
    pub const PATH_UNLINK_FILE: Self = Self(1 << 26);

    /// Rights inheritable by files opened from a directory.
    pub const OPEN_FILE_BASE: Self = Self(
        Self::FD_READ.0
            | Self::FD_SEEK.0
            | Self::FD_TELL.0
            | Self::FD_WRITE.0
            | Self::FD_FILESTAT_GET.0
            | Self::FD_FILESTAT_SET_TIMES.0,
    );

    /// Rights for directory fds that can resolve namespace paths.
    pub const DIRECTORY_BASE: Self = Self(
        Self::PATH_CREATE_DIRECTORY.0
            | Self::PATH_CREATE_FILE.0
            | Self::PATH_OPEN.0
            | Self::FD_READDIR.0
            | Self::PATH_RENAME_SOURCE.0
            | Self::PATH_RENAME_TARGET.0
            | Self::PATH_FILESTAT_GET.0
            | Self::PATH_FILESTAT_SET_SIZE.0
            | Self::PATH_FILESTAT_SET_TIMES.0
            | Self::FD_FILESTAT_GET.0
            | Self::FD_FILESTAT_SET_TIMES.0
            | Self::PATH_REMOVE_DIRECTORY.0
            | Self::PATH_UNLINK_FILE.0,
    );

    /// Rights inheritable by files or directories opened from a directory.
    pub const DIRECTORY_INHERITING: Self = Self(Self::DIRECTORY_BASE.0 | Self::OPEN_FILE_BASE.0);

    /// Returns the raw WASI Preview 1 rights bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Creates rights from raw WASI Preview 1 bits.
    #[must_use]
    pub const fn from_preview1_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns whether all `right` bits are set.
    #[must_use]
    pub const fn contains(self, right: Self) -> bool {
        self.0 & right.0 == right.0
    }

    /// Returns the rights present in both sets.
    #[must_use]
    pub const fn intersection(self, rights: Self) -> Self {
        Self(self.0 & rights.0)
    }

    /// Returns whether no rights are set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for WasiRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for WasiRights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Preview 1 timestamp flags for `path_filestat_set_times`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiFilestatSetTimes {
    flags: u16,
}

impl WasiFilestatSetTimes {
    /// Use the explicit access-time timestamp argument.
    pub const ATIM: u16 = 1 << 0;
    /// Set access time to the host's current time.
    pub const ATIM_NOW: u16 = 1 << 1;
    /// Use the explicit modification-time timestamp argument.
    pub const MTIM: u16 = 1 << 2;
    /// Set modification time to the host's current time.
    pub const MTIM_NOW: u16 = 1 << 3;

    const SUPPORTED: u16 = Self::ATIM | Self::ATIM_NOW | Self::MTIM | Self::MTIM_NOW;

    /// Converts raw Preview 1 flags into a validated set.
    pub fn from_preview1(flags: u16) -> Result<Self, Errno> {
        if flags & !Self::SUPPORTED != 0 {
            return Err(Errno::Inval);
        }
        if flags & (Self::ATIM | Self::ATIM_NOW) == Self::ATIM | Self::ATIM_NOW {
            return Err(Errno::Inval);
        }
        if flags & (Self::MTIM | Self::MTIM_NOW) == Self::MTIM | Self::MTIM_NOW {
            return Err(Errno::Inval);
        }
        if flags & (Self::ATIM_NOW | Self::MTIM_NOW) != 0 {
            return Err(Errno::Notcapable);
        }
        Ok(Self { flags })
    }

    /// Returns whether the access time should be set from the timestamp argument.
    #[must_use]
    pub const fn set_access_time(self) -> bool {
        self.flags & Self::ATIM != 0
    }

    /// Returns whether the modification time should be set from the timestamp argument.
    #[must_use]
    pub const fn set_modified_time(self) -> bool {
        self.flags & Self::MTIM != 0
    }

    /// Returns whether no explicit timestamp updates were requested.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.flags == 0
    }
}

/// WASI Preview 1 fdstat-like metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiFdStat {
    file_type: WasiFileType,
    fdflags: u16,
    rights_base: WasiRights,
    rights_inheriting: WasiRights,
}

impl WasiFdStat {
    /// Byte size of a WASI Preview 1 `fdstat` record.
    pub const PREVIEW1_SIZE: usize = 24;

    pub(crate) const fn new(
        file_type: WasiFileType,
        rights_base: WasiRights,
        rights_inheriting: WasiRights,
    ) -> Self {
        Self::new_with_fdflags(file_type, 0, rights_base, rights_inheriting)
    }

    pub(crate) const fn new_with_fdflags(
        file_type: WasiFileType,
        fdflags: u16,
        rights_base: WasiRights,
        rights_inheriting: WasiRights,
    ) -> Self {
        Self {
            file_type,
            fdflags,
            rights_base,
            rights_inheriting,
        }
    }

    /// Returns the WASI file type.
    #[must_use]
    pub const fn file_type(&self) -> WasiFileType {
        self.file_type
    }

    /// Returns Preview 1 fdflags for this fd.
    #[must_use]
    pub const fn fdflags(&self) -> u16 {
        self.fdflags
    }

    /// Returns rights that apply to this fd.
    #[must_use]
    pub const fn rights_base(&self) -> WasiRights {
        self.rights_base
    }

    /// Returns rights inherited by fds opened from this fd.
    #[must_use]
    pub const fn rights_inheriting(&self) -> WasiRights {
        self.rights_inheriting
    }

    /// Encodes this fdstat using the WASI Preview 1 little-endian layout.
    #[must_use]
    pub fn to_preview1_bytes(self) -> [u8; Self::PREVIEW1_SIZE] {
        let mut bytes = [0; Self::PREVIEW1_SIZE];
        bytes[0] = self.file_type.preview1_code();
        bytes[2..4].copy_from_slice(&self.fdflags.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.rights_base.bits().to_le_bytes());
        bytes[16..24].copy_from_slice(&self.rights_inheriting.bits().to_le_bytes());
        bytes
    }
}

/// WASI Preview 1 prestat-like metadata for preopened directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasiPrestat {
    dir_name: String,
}

impl WasiPrestat {
    /// Byte size of a WASI Preview 1 `prestat` record.
    pub const PREVIEW1_SIZE: usize = 8;

    pub(crate) fn from_path(path: &NormalizedPath) -> Self {
        let dir_name = if path.as_str() == "." {
            "/".to_owned()
        } else {
            format!("/{}", path.as_str())
        };
        Self { dir_name }
    }

    /// Returns the guest directory name reported by Preview 1 prestat calls.
    #[must_use]
    pub fn dir_name(&self) -> &str {
        &self.dir_name
    }

    /// Returns the guest directory name length in bytes.
    #[must_use]
    pub fn dir_name_len(&self) -> usize {
        self.dir_name.len()
    }

    /// Encodes this prestat using the WASI Preview 1 little-endian layout.
    #[must_use]
    pub fn to_preview1_bytes(&self) -> [u8; Self::PREVIEW1_SIZE] {
        let mut bytes = [0; Self::PREVIEW1_SIZE];
        let len = u32::try_from(self.dir_name.len()).expect("WASI preopen name fits u32");
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        bytes
    }
}

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
        if !WasiRights::DIRECTORY_INHERITING.contains(rights_base)
            || !WasiRights::DIRECTORY_INHERITING.contains(rights_inheriting)
        {
            return Err(Errno::Notcapable);
        }
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

/// Read/write rights for files attached directly to WASI fds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiFileAccess {
    read: bool,
    write: bool,
    append: bool,
}

impl WasiFileAccess {
    /// Creates attached fd access from explicit read/write capabilities.
    #[must_use]
    pub const fn new(read: bool, write: bool) -> Self {
        Self {
            read,
            write,
            append: false,
        }
    }

    /// Returns this access policy with append writes enabled or disabled.
    #[must_use]
    pub const fn with_append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    /// Read-only attached fd access.
    #[must_use]
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            append: false,
        }
    }

    /// Write-only attached fd access.
    #[must_use]
    pub fn write_only() -> Self {
        Self {
            read: false,
            write: true,
            append: false,
        }
    }

    /// Read-write attached fd access.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            append: false,
        }
    }

    pub(crate) fn can_read(self) -> bool {
        self.read
    }

    pub(crate) fn can_write(self) -> bool {
        self.write
    }

    pub(crate) fn append(self) -> bool {
        self.append
    }
}

/// Stat result returned by the initial WASI context API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    file_type: FileType,
    len: u64,
    mode: u32,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl FileStat {
    /// Byte size of a WASI Preview 1 `filestat` record.
    pub const PREVIEW1_SIZE: usize = 64;

    pub(crate) fn new(metadata: wanix_fs::Metadata) -> Self {
        Self {
            file_type: metadata.file_type(),
            len: metadata.len(),
            mode: metadata.mode(),
            accessed_time_ns: metadata.accessed_time_ns(),
            modified_time_ns: metadata.modified_time_ns(),
            changed_time_ns: metadata.changed_time_ns(),
        }
    }

    /// Returns the Wanix file type.
    #[must_use]
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the corresponding WASI Preview 1 file type.
    #[must_use]
    pub const fn wasi_file_type(&self) -> WasiFileType {
        WasiFileType::from_wanix(self.file_type)
    }

    /// Returns the byte length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns whether the stat reports no byte content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the Unix-style mode.
    #[must_use]
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Returns the access timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn accessed_time_ns(&self) -> u64 {
        self.accessed_time_ns
    }

    /// Returns the modification timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn modified_time_ns(&self) -> u64 {
        self.modified_time_ns
    }

    /// Returns the metadata-change timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn changed_time_ns(&self) -> u64 {
        self.changed_time_ns
    }

    /// Encodes this filestat using the WASI Preview 1 little-endian layout.
    #[must_use]
    pub fn to_preview1_bytes(&self) -> [u8; Self::PREVIEW1_SIZE] {
        let mut bytes = [0; Self::PREVIEW1_SIZE];
        bytes[16] = self.wasi_file_type().preview1_code();
        bytes[32..40].copy_from_slice(&self.len.to_le_bytes());
        bytes[40..48].copy_from_slice(&self.accessed_time_ns.to_le_bytes());
        bytes[48..56].copy_from_slice(&self.modified_time_ns.to_le_bytes());
        bytes[56..64].copy_from_slice(&self.changed_time_ns.to_le_bytes());
        bytes
    }
}
