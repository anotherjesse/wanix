/// Decoded payload for `Rlerror`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Lerror {
    /// Linux errno value reported by a 9P2000.L server.
    pub ecode: u32,
}

/// Decoded payload for `Tstatfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9StatFs {
    /// Fid whose filesystem should be reported.
    pub fid: u32,
}

/// Decoded payload for `Tfsync`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Fsync {
    /// Fid to synchronize.
    pub fid: u32,
}

/// Decoded payload for `Tflush`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Flush {
    /// Tag of the request being flushed.
    pub oldtag: u16,
}

/// Decoded payload for `Tflushf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9FlushF {
    /// Fid whose pending file state should be flushed.
    pub fid: u32,
}

/// 9P2000.L record-lock range fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Lock {
    /// Lock kind: read, write, or unlock.
    pub lock_type: u8,
    /// Starting byte offset for the lock range.
    pub start: u64,
    /// Number of bytes in the lock range.
    pub length: u64,
    /// Process id associated with the lock request.
    pub proc_id: u32,
    /// Client id string, usually the Linux v9fs client nodename.
    pub client_id: String,
}

/// Decoded payload for `Tlock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9LockRequest {
    /// Fid to lock or unlock.
    pub fid: u32,
    /// 9P2000.L lock flags.
    pub flags: u32,
    /// Requested lock range.
    pub lock: P9Lock,
}

/// Decoded payload for `Tgetlock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9GetLockRequest {
    /// Fid whose advisory-lock state should be queried.
    pub fid: u32,
    /// Requested lock range.
    pub lock: P9Lock,
}

/// 9P2000.L filesystem stats returned by `Rstatfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9FsStat {
    /// Filesystem type magic.
    pub fs_type: u32,
    /// Filesystem block size.
    pub block_size: u32,
    /// Total data blocks.
    pub blocks: u64,
    /// Free data blocks.
    pub blocks_free: u64,
    /// Free blocks available to unprivileged users.
    pub blocks_available: u64,
    /// Total file nodes.
    pub files: u64,
    /// Free file nodes.
    pub files_free: u64,
    /// Filesystem id.
    pub fsid: u64,
    /// Maximum filename length.
    pub name_length: u32,
}
