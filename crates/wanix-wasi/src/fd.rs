use std::ops::{BitOr, BitOrAssign};

use wanix_fs::{FileType, NormalizedPath};

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
    /// Create a file at a path relative to this fd.
    pub const PATH_CREATE_FILE: Self = Self(1 << 10);
    /// Open a path relative to this fd.
    pub const PATH_OPEN: Self = Self(1 << 13);
    /// Read directory entries from an fd.
    pub const FD_READDIR: Self = Self(1 << 14);
    /// Stat a path relative to this fd.
    pub const PATH_FILESTAT_GET: Self = Self(1 << 18);
    /// Set the size of a path relative to this fd.
    pub const PATH_FILESTAT_SET_SIZE: Self = Self(1 << 19);
    /// Stat this fd.
    pub const FD_FILESTAT_GET: Self = Self(1 << 21);

    /// Rights inheritable by files opened from a directory.
    pub const OPEN_FILE_BASE: Self =
        Self(Self::FD_READ.0 | Self::FD_WRITE.0 | Self::FD_FILESTAT_GET.0);

    /// Rights for directory fds that can resolve namespace paths.
    pub const DIRECTORY_BASE: Self = Self(
        Self::PATH_CREATE_FILE.0
            | Self::PATH_OPEN.0
            | Self::FD_READDIR.0
            | Self::PATH_FILESTAT_GET.0
            | Self::PATH_FILESTAT_SET_SIZE.0
            | Self::FD_FILESTAT_GET.0,
    );

    /// Rights inheritable by files or directories opened from a directory.
    pub const DIRECTORY_INHERITING: Self = Self(Self::DIRECTORY_BASE.0 | Self::OPEN_FILE_BASE.0);

    /// Returns the raw WASI Preview 1 rights bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Returns whether all `right` bits are set.
    #[must_use]
    pub const fn contains(self, right: Self) -> bool {
        self.0 & right.0 == right.0
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

/// WASI Preview 1 fdstat-like metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiFdStat {
    file_type: WasiFileType,
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
        Self {
            file_type,
            rights_base,
            rights_inheriting,
        }
    }

    /// Returns the WASI file type.
    #[must_use]
    pub const fn file_type(&self) -> WasiFileType {
        self.file_type
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
}

impl WasiOpenOptions {
    /// Read-only file open.
    #[must_use]
    pub fn read() -> Self {
        Self {
            read: true,
            write: false,
            create: false,
            truncate: false,
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
        }
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
}

impl WasiFileAccess {
    /// Read-only attached fd access.
    #[must_use]
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
        }
    }

    /// Write-only attached fd access.
    #[must_use]
    pub fn write_only() -> Self {
        Self {
            read: false,
            write: true,
        }
    }

    /// Read-write attached fd access.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
        }
    }

    pub(crate) fn can_read(self) -> bool {
        self.read
    }

    pub(crate) fn can_write(self) -> bool {
        self.write
    }
}

/// Stat result returned by the initial WASI context API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    file_type: FileType,
    len: u64,
    mode: u32,
}

impl FileStat {
    pub(crate) fn new(metadata: wanix_fs::Metadata) -> Self {
        Self {
            file_type: metadata.file_type(),
            len: metadata.len(),
            mode: metadata.mode(),
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
}
