use wanix_fs::{FileType, NormalizedPath};

use super::WasiRights;

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

    const fn from_wanix(file_type: FileType) -> Self {
        match file_type {
            FileType::File => Self::RegularFile,
            FileType::Directory => Self::Directory,
            FileType::Symlink => Self::SymbolicLink,
        }
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
