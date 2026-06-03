use wanix_fs::FileType;

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
