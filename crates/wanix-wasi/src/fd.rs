mod flags;
mod open;
mod rights;
mod stat;

pub use flags::{WasiFilestatSetTimes, WasiLookupFlags};
pub use open::{WasiOpenOptions, WasiPathOpen};
pub use rights::WasiRights;
pub use stat::{FileStat, WasiFd, WasiFdStat, WasiFileType, WasiPrestat};

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
