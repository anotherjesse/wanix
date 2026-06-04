use super::types::P9Qid;

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
