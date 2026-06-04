use std::ops::{BitOr, BitOrAssign};

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
    /// Read a symbolic link target at a path relative to this fd.
    pub const PATH_READLINK: Self = Self(1 << 15);
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
    /// Set the size of this fd.
    pub const FD_FILESTAT_SET_SIZE: Self = Self(1 << 22);
    /// Set access or modification times on this fd.
    pub const FD_FILESTAT_SET_TIMES: Self = Self(1 << 23);
    /// Create a symbolic link at a path relative to this fd.
    pub const PATH_SYMLINK: Self = Self(1 << 24);
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
            | Self::FD_FILESTAT_SET_SIZE.0
            | Self::FD_FILESTAT_SET_TIMES.0,
    );

    /// Rights for directory fds that can resolve namespace paths.
    pub const DIRECTORY_BASE: Self = Self(
        Self::PATH_CREATE_DIRECTORY.0
            | Self::PATH_CREATE_FILE.0
            | Self::PATH_OPEN.0
            | Self::FD_READDIR.0
            | Self::PATH_READLINK.0
            | Self::PATH_RENAME_SOURCE.0
            | Self::PATH_RENAME_TARGET.0
            | Self::PATH_FILESTAT_GET.0
            | Self::PATH_FILESTAT_SET_SIZE.0
            | Self::PATH_FILESTAT_SET_TIMES.0
            | Self::FD_FILESTAT_GET.0
            | Self::FD_FILESTAT_SET_TIMES.0
            | Self::PATH_SYMLINK.0
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
