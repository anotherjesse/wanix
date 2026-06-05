use super::session::P9Qid;

/// Decoded payload for `Tread`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Read {
    /// Fid to read from.
    pub fid: u32,
    /// Offset to read from.
    pub offset: u64,
    /// Maximum number of bytes requested.
    pub count: u32,
}

/// Decoded payload for `Treaddir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9ReadDir {
    /// Fid to read directory entries from.
    pub fid: u32,
    /// Opaque directory offset cookie supplied by a previous entry.
    pub offset: u64,
    /// Maximum number of directory-entry bytes requested.
    pub count: u32,
}

/// One 9P2000.L directory entry record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9DirEntry {
    /// QID for the listed child.
    pub qid: P9Qid,
    /// Opaque cookie for the next read position.
    pub offset: u64,
    /// Linux `DT_*` directory entry type.
    pub dirent_type: u8,
    /// Child basename.
    pub name: String,
}

/// Decoded payload for `Twrite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Write {
    /// Fid to write to.
    pub fid: u32,
    /// Offset to write at.
    pub offset: u64,
    /// Bytes to write.
    pub data: Vec<u8>,
}

/// Decoded payload for `Tclunk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Clunk {
    /// Fid to release.
    pub fid: u32,
}
