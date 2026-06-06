use super::types::P9Qid;

/// `readdir` directory-entry type for a directory.
pub const DT_DIR: u8 = 4;
/// `readdir` directory-entry type for a regular file.
pub const DT_REG: u8 = 8;
/// `readdir` directory-entry type for a symbolic link.
pub const DT_LNK: u8 = 10;

/// Mask selecting the POSIX file-type bits of a mode word.
pub const P9_MODE_TYPE_MASK: u32 = 0o170000;
/// POSIX mode file-type bits for a directory.
pub const P9_MODE_DIR: u32 = 0o040000;
/// POSIX mode file-type bits for a regular file.
pub const P9_MODE_REG: u32 = 0o100000;
/// POSIX mode file-type bits for a symbolic link.
pub const P9_MODE_LNK: u32 = 0o120000;

/// QID type byte for a directory.
pub const P9_QID_TYPE_DIR: u8 = 0x80;
/// QID type byte for a symbolic link.
pub const P9_QID_TYPE_SYMLINK: u8 = 0x02;
/// QID type byte for a regular file.
pub const P9_QID_TYPE_FILE: u8 = 0;

/// Preferred block size reported for Wanix files in `Rgetattr`/`Rstatfs`.
pub const P9_DEFAULT_BLOCK_SIZE: u64 = 65_536;
/// Synthetic filesystem magic reported by `Rstatfs`.
pub const P9_FS_MAGIC: u32 = 0x0102_1997;
/// Maximum filename length reported by `Rstatfs`.
pub const P9_DEFAULT_NAME_LENGTH: u32 = 255;

/// Returns the `readdir` directory-entry type byte for a QID type byte.
///
/// Directories and symbolic links map to their dedicated entry types; every
/// other QID type is reported as a regular file.
#[must_use]
pub fn p9_dirent_type_for_qid_type(qid_type: u8) -> u8 {
    match qid_type {
        P9_QID_TYPE_DIR => DT_DIR,
        P9_QID_TYPE_SYMLINK => DT_LNK,
        _ => DT_REG,
    }
}

/// Returns the POSIX mode file-type bits for a QID type byte.
///
/// Directories and symbolic links map to their dedicated mode bits; every other
/// QID type is reported as a regular file.
#[must_use]
pub fn p9_mode_type_for_qid_type(qid_type: u8) -> u32 {
    match qid_type {
        P9_QID_TYPE_DIR => P9_MODE_DIR,
        P9_QID_TYPE_SYMLINK => P9_MODE_LNK,
        _ => P9_MODE_REG,
    }
}

/// 9P2000.L attribute payload returned by `Rgetattr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Attr {
    /// Attribute bits the server considers valid.
    pub valid: u64,
    /// QID for the file.
    pub qid: P9Qid,
    /// POSIX mode including file type bits.
    pub mode: u32,
    /// Numeric owner user id.
    pub uid: u32,
    /// Numeric owner group id.
    pub gid: u32,
    /// Link count.
    pub nlink: u64,
    /// Device id for special files.
    pub rdev: u64,
    /// File size in bytes.
    pub size: u64,
    /// Preferred block size.
    pub block_size: u64,
    /// Allocated 512-byte block count.
    pub blocks: u64,
    /// Last access time seconds.
    pub atime_seconds: u64,
    /// Last access time nanoseconds.
    pub atime_nanoseconds: u64,
    /// Last modification time seconds.
    pub mtime_seconds: u64,
    /// Last modification time nanoseconds.
    pub mtime_nanoseconds: u64,
    /// Last metadata-change time seconds.
    pub ctime_seconds: u64,
    /// Last metadata-change time nanoseconds.
    pub ctime_nanoseconds: u64,
    /// Creation/birth time seconds.
    pub btime_seconds: u64,
    /// Creation/birth time nanoseconds.
    pub btime_nanoseconds: u64,
    /// File generation value.
    pub generation: u64,
    /// Server data-version value.
    pub data_version: u64,
}

/// 9P attribute body without `valid` or `qid`, used by `Rwalkgetattr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9AttrBody {
    /// POSIX mode including file type bits.
    pub mode: u32,
    /// Numeric owner user id.
    pub uid: u32,
    /// Numeric owner group id.
    pub gid: u32,
    /// Link count.
    pub nlink: u64,
    /// Device id for special files.
    pub rdev: u64,
    /// File size in bytes.
    pub size: u64,
    /// Preferred block size.
    pub block_size: u64,
    /// Allocated 512-byte block count.
    pub blocks: u64,
    /// Last access time seconds.
    pub atime_seconds: u64,
    /// Last access time nanoseconds.
    pub atime_nanoseconds: u64,
    /// Last modification time seconds.
    pub mtime_seconds: u64,
    /// Last modification time nanoseconds.
    pub mtime_nanoseconds: u64,
    /// Last metadata-change time seconds.
    pub ctime_seconds: u64,
    /// Last metadata-change time nanoseconds.
    pub ctime_nanoseconds: u64,
    /// Creation/birth time seconds.
    pub btime_seconds: u64,
    /// Creation/birth time nanoseconds.
    pub btime_nanoseconds: u64,
    /// File generation value.
    pub generation: u64,
    /// Server data-version value.
    pub data_version: u64,
}

impl From<&P9Attr> for P9AttrBody {
    fn from(attr: &P9Attr) -> Self {
        Self {
            mode: attr.mode,
            uid: attr.uid,
            gid: attr.gid,
            nlink: attr.nlink,
            rdev: attr.rdev,
            size: attr.size,
            block_size: attr.block_size,
            blocks: attr.blocks,
            atime_seconds: attr.atime_seconds,
            atime_nanoseconds: attr.atime_nanoseconds,
            mtime_seconds: attr.mtime_seconds,
            mtime_nanoseconds: attr.mtime_nanoseconds,
            ctime_seconds: attr.ctime_seconds,
            ctime_nanoseconds: attr.ctime_nanoseconds,
            btime_seconds: attr.btime_seconds,
            btime_nanoseconds: attr.btime_nanoseconds,
            generation: attr.generation,
            data_version: attr.data_version,
        }
    }
}

/// 9P2000.L attribute values carried by `Tsetattr`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct P9SetAttr {
    /// POSIX permission bits requested by `P9_SETATTR_PERMISSIONS`.
    pub permissions: u32,
    /// Numeric owner user id requested by `P9_SETATTR_UID`.
    pub uid: u32,
    /// Numeric owner group id requested by `P9_SETATTR_GID`.
    pub gid: u32,
    /// File size requested by `P9_SETATTR_SIZE`.
    pub size: u64,
    /// Explicit access time seconds.
    pub atime_seconds: u64,
    /// Explicit access time nanoseconds.
    pub atime_nanoseconds: u64,
    /// Explicit modification time seconds.
    pub mtime_seconds: u64,
    /// Explicit modification time nanoseconds.
    pub mtime_nanoseconds: u64,
}

/// Decoded payload for `Tsetattr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9SetAttrRequest {
    /// Fid whose attributes should change.
    pub fid: u32,
    /// Raw 9P2000.L setattr valid mask.
    pub valid: u32,
    /// Requested attribute values.
    pub attr: P9SetAttr,
}
