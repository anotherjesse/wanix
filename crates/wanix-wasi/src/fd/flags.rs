use wanix_fs::MetadataLookup;

use crate::Errno;

/// WASI Preview 1 lookup flags for path metadata and mutation calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiLookupFlags(u32);

impl WasiLookupFlags {
    /// Follow the final symlink component while resolving a path.
    pub const SYMLINK_FOLLOW: u32 = 1 << 0;
    const SUPPORTED: u32 = Self::SYMLINK_FOLLOW;

    /// Converts raw Preview 1 lookup flags into a validated set.
    pub fn from_preview1(flags: u32) -> Result<Self, Errno> {
        if flags & !Self::SUPPORTED != 0 {
            return Err(Errno::Notcapable);
        }
        Ok(Self(flags))
    }

    /// Returns whether the final symlink component should be followed.
    #[must_use]
    pub const fn follow_symlinks(self) -> bool {
        self.0 & Self::SYMLINK_FOLLOW != 0
    }

    pub(crate) const fn metadata_lookup(self) -> MetadataLookup {
        if self.follow_symlinks() {
            MetadataLookup::FollowSymlink
        } else {
            MetadataLookup::NoFollow
        }
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
        Ok(Self { flags })
    }

    /// Returns whether the access time should be set from the timestamp argument.
    #[must_use]
    pub const fn set_access_time(self) -> bool {
        self.flags & Self::ATIM != 0
    }

    /// Returns whether the access time should be set to the configured clock time.
    #[must_use]
    pub const fn set_access_time_to_now(self) -> bool {
        self.flags & Self::ATIM_NOW != 0
    }

    /// Returns whether the modification time should be set from the timestamp argument.
    #[must_use]
    pub const fn set_modified_time(self) -> bool {
        self.flags & Self::MTIM != 0
    }

    /// Returns whether the modification time should be set to the configured clock time.
    #[must_use]
    pub const fn set_modified_time_to_now(self) -> bool {
        self.flags & Self::MTIM_NOW != 0
    }

    /// Returns whether no explicit timestamp updates were requested.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.flags == 0
    }
}
