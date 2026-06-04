//! 9P wire protocol helpers.
//!
//! This module is intentionally below Wanix filesystem policy: callers can
//! split byte streams into tagged 9P frames and decode typed payloads before
//! the Rust port grows a full 9P server backed by Wanix namespaces.

use std::fmt;

/// 9P2000.L version string used by the existing v86 integration.
pub const P9_VERSION_9P2000_L: &str = "9P2000.L";

/// The 9P `NOTAG` value used by version negotiation.
pub const P9_NOTAG: u16 = 0xffff;

/// The 9P `NOFID` value used when no auth fid is supplied.
pub const P9_NOFID: u32 = 0xffff_ffff;

/// Minimum 9P frame size: `size[4] type[1] tag[2]`.
pub const P9_HEADER_LEN: usize = 7;

/// 9P `Rlerror` message type.
pub const P9_RLERROR: u8 = 7;

/// 9P2000.L `Tstatfs` message type.
pub const P9_TSTATFS: u8 = 8;

/// 9P2000.L `Rstatfs` message type.
pub const P9_RSTATFS: u8 = 9;

/// 9P2000.L `Tlopen` message type.
pub const P9_TLOPEN: u8 = 12;

/// 9P2000.L `Rlopen` message type.
pub const P9_RLOPEN: u8 = 13;

/// 9P2000.L `Tlcreate` message type.
pub const P9_TLCREATE: u8 = 14;

/// 9P2000.L `Rlcreate` message type.
pub const P9_RLCREATE: u8 = 15;

/// 9P2000.L `Tsymlink` message type.
pub const P9_TSYMLINK: u8 = 16;

/// 9P2000.L `Rsymlink` message type.
pub const P9_RSYMLINK: u8 = 17;

/// 9P2000.L `Tmknod` message type.
pub const P9_TMKNOD: u8 = 18;

/// 9P2000.L `Rmknod` message type.
pub const P9_RMKNOD: u8 = 19;

/// 9P2000.L `Trename` message type.
pub const P9_TRENAME: u8 = 20;

/// 9P2000.L `Rrename` message type.
pub const P9_RRENAME: u8 = 21;

/// 9P2000.L `Treadlink` message type.
pub const P9_TREADLINK: u8 = 22;

/// 9P2000.L `Rreadlink` message type.
pub const P9_RREADLINK: u8 = 23;

/// 9P2000.L `Tgetattr` message type.
pub const P9_TGETATTR: u8 = 24;

/// 9P2000.L `Rgetattr` message type.
pub const P9_RGETATTR: u8 = 25;

/// 9P2000.L `Tsetattr` message type.
pub const P9_TSETATTR: u8 = 26;

/// 9P2000.L `Rsetattr` message type.
pub const P9_RSETATTR: u8 = 27;

/// 9P2000.L `Txattrwalk` message type.
pub const P9_TXATTRWALK: u8 = 30;

/// 9P2000.L `Rxattrwalk` message type.
pub const P9_RXATTRWALK: u8 = 31;

/// 9P2000.L `Txattrcreate` message type.
pub const P9_TXATTRCREATE: u8 = 32;

/// 9P2000.L `Rxattrcreate` message type.
pub const P9_RXATTRCREATE: u8 = 33;

/// 9P2000.L `Treaddir` message type.
pub const P9_TREADDIR: u8 = 40;

/// 9P2000.L `Rreaddir` message type.
pub const P9_RREADDIR: u8 = 41;

/// 9P2000.L `Tfsync` message type.
pub const P9_TFSYNC: u8 = 50;

/// 9P2000.L `Rfsync` message type.
pub const P9_RFSYNC: u8 = 51;

/// 9P2000.L `Tlock` message type.
pub const P9_TLOCK: u8 = 52;

/// 9P2000.L `Rlock` message type.
pub const P9_RLOCK: u8 = 53;

/// 9P2000.L `Tgetlock` message type.
pub const P9_TGETLOCK: u8 = 54;

/// 9P2000.L `Rgetlock` message type.
pub const P9_RGETLOCK: u8 = 55;

/// 9P2000.L `Tlink` message type.
pub const P9_TLINK: u8 = 70;

/// 9P2000.L `Rlink` message type.
pub const P9_RLINK: u8 = 71;

/// 9P2000.L `Tmkdir` message type.
pub const P9_TMKDIR: u8 = 72;

/// 9P2000.L `Rmkdir` message type.
pub const P9_RMKDIR: u8 = 73;

/// 9P2000.L `Trenameat` message type.
pub const P9_TRENAMEAT: u8 = 74;

/// 9P2000.L `Rrenameat` message type.
pub const P9_RRENAMEAT: u8 = 75;

/// 9P2000.L `Tunlinkat` message type.
pub const P9_TUNLINKAT: u8 = 76;

/// 9P2000.L `Runlinkat` message type.
pub const P9_RUNLINKAT: u8 = 77;

/// 9P `Tversion` message type.
pub const P9_TVERSION: u8 = 100;

/// 9P `Rversion` message type.
pub const P9_RVERSION: u8 = 101;

/// 9P `Tauth` message type.
pub const P9_TAUTH: u8 = 102;

/// 9P `Rauth` message type.
pub const P9_RAUTH: u8 = 103;

/// 9P `Tattach` message type.
pub const P9_TATTACH: u8 = 104;

/// 9P `Rattach` message type.
pub const P9_RATTACH: u8 = 105;

/// 9P `Tflush` message type.
pub const P9_TFLUSH: u8 = 108;

/// 9P `Rflush` message type.
pub const P9_RFLUSH: u8 = 109;

/// 9P `Twalk` message type.
pub const P9_TWALK: u8 = 110;

/// 9P `Rwalk` message type.
pub const P9_RWALK: u8 = 111;

/// 9P `Tread` message type.
pub const P9_TREAD: u8 = 116;

/// 9P `Rread` message type.
pub const P9_RREAD: u8 = 117;

/// 9P `Twrite` message type.
pub const P9_TWRITE: u8 = 118;

/// 9P `Rwrite` message type.
pub const P9_RWRITE: u8 = 119;

/// 9P `Tclunk` message type.
pub const P9_TCLUNK: u8 = 120;

/// 9P `Rclunk` message type.
pub const P9_RCLUNK: u8 = 121;

/// 9P `Tremove` message type.
pub const P9_TREMOVE: u8 = 122;

/// 9P `Rremove` message type.
pub const P9_RREMOVE: u8 = 123;

/// 9P2000.L `Tsetattr` permissions-valid bit.
pub const P9_SETATTR_PERMISSIONS: u32 = 0x0000_0001;

/// 9P2000.L `Tsetattr` uid-valid bit.
pub const P9_SETATTR_UID: u32 = 0x0000_0002;

/// 9P2000.L `Tsetattr` gid-valid bit.
pub const P9_SETATTR_GID: u32 = 0x0000_0004;

/// 9P2000.L `Tsetattr` size-valid bit.
pub const P9_SETATTR_SIZE: u32 = 0x0000_0008;

/// 9P2000.L `Tsetattr` access-time-valid bit.
pub const P9_SETATTR_ATIME: u32 = 0x0000_0010;

/// 9P2000.L `Tsetattr` modification-time-valid bit.
pub const P9_SETATTR_MTIME: u32 = 0x0000_0020;

/// 9P2000.L `Tsetattr` metadata-change-time-valid bit.
pub const P9_SETATTR_CTIME: u32 = 0x0000_0040;

/// 9P2000.L `Tsetattr` access time is explicit rather than server current time.
pub const P9_SETATTR_ATIME_NOT_SYSTEM_TIME: u32 = 0x0000_0080;

/// 9P2000.L `Tsetattr` modification time is explicit rather than server current time.
pub const P9_SETATTR_MTIME_NOT_SYSTEM_TIME: u32 = 0x0000_0100;

/// 9P2000.L read-lock type.
pub const P9_LOCK_TYPE_READ: u8 = 0;

/// 9P2000.L write-lock type.
pub const P9_LOCK_TYPE_WRITE: u8 = 1;

/// 9P2000.L unlock/no-conflict type.
pub const P9_LOCK_TYPE_UNLOCK: u8 = 2;

/// 9P2000.L lock request succeeded.
pub const P9_LOCK_STATUS_OK: u8 = 0;

/// 9P2000.L lock request blocked.
pub const P9_LOCK_STATUS_BLOCKED: u8 = 1;

/// 9P2000.L lock request failed.
pub const P9_LOCK_STATUS_ERROR: u8 = 2;

/// 9P2000.L lock request is in grace period.
pub const P9_LOCK_STATUS_GRACE: u8 = 3;

/// Returns a stable name for message types the Rust port currently identifies.
#[must_use]
pub const fn p9_message_type_name(message_type: u8) -> Option<&'static str> {
    match message_type {
        P9_RLERROR => Some("Rlerror"),
        P9_TSTATFS => Some("Tstatfs"),
        P9_RSTATFS => Some("Rstatfs"),
        P9_TLOPEN => Some("Tlopen"),
        P9_RLOPEN => Some("Rlopen"),
        P9_TLCREATE => Some("Tlcreate"),
        P9_RLCREATE => Some("Rlcreate"),
        P9_TSYMLINK => Some("Tsymlink"),
        P9_RSYMLINK => Some("Rsymlink"),
        P9_TMKNOD => Some("Tmknod"),
        P9_RMKNOD => Some("Rmknod"),
        P9_TRENAME => Some("Trename"),
        P9_RRENAME => Some("Rrename"),
        P9_TREADLINK => Some("Treadlink"),
        P9_RREADLINK => Some("Rreadlink"),
        P9_TGETATTR => Some("Tgetattr"),
        P9_RGETATTR => Some("Rgetattr"),
        P9_TSETATTR => Some("Tsetattr"),
        P9_RSETATTR => Some("Rsetattr"),
        P9_TXATTRWALK => Some("Txattrwalk"),
        P9_RXATTRWALK => Some("Rxattrwalk"),
        P9_TXATTRCREATE => Some("Txattrcreate"),
        P9_RXATTRCREATE => Some("Rxattrcreate"),
        P9_TREADDIR => Some("Treaddir"),
        P9_RREADDIR => Some("Rreaddir"),
        P9_TFSYNC => Some("Tfsync"),
        P9_RFSYNC => Some("Rfsync"),
        P9_TLOCK => Some("Tlock"),
        P9_RLOCK => Some("Rlock"),
        P9_TGETLOCK => Some("Tgetlock"),
        P9_RGETLOCK => Some("Rgetlock"),
        P9_TLINK => Some("Tlink"),
        P9_RLINK => Some("Rlink"),
        P9_TMKDIR => Some("Tmkdir"),
        P9_RMKDIR => Some("Rmkdir"),
        P9_TRENAMEAT => Some("Trenameat"),
        P9_RRENAMEAT => Some("Rrenameat"),
        P9_TUNLINKAT => Some("Tunlinkat"),
        P9_RUNLINKAT => Some("Runlinkat"),
        P9_TVERSION => Some("Tversion"),
        P9_RVERSION => Some("Rversion"),
        P9_TAUTH => Some("Tauth"),
        P9_RAUTH => Some("Rauth"),
        P9_TATTACH => Some("Tattach"),
        P9_RATTACH => Some("Rattach"),
        P9_TFLUSH => Some("Tflush"),
        P9_RFLUSH => Some("Rflush"),
        P9_TWALK => Some("Twalk"),
        P9_RWALK => Some("Rwalk"),
        P9_TREAD => Some("Tread"),
        P9_RREAD => Some("Rread"),
        P9_TWRITE => Some("Twrite"),
        P9_RWRITE => Some("Rwrite"),
        P9_TCLUNK => Some("Tclunk"),
        P9_RCLUNK => Some("Rclunk"),
        P9_TREMOVE => Some("Tremove"),
        P9_RREMOVE => Some("Rremove"),
        _ => None,
    }
}

/// 9P frame or payload decode/encode error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P9Error {
    /// The input is shorter than the 9P frame header.
    ShortFrame {
        /// Actual number of bytes supplied.
        len: usize,
    },
    /// The declared frame size is smaller than the 9P header.
    InvalidFrameSize {
        /// Declared frame size in bytes.
        size: usize,
    },
    /// The declared size does not match the exact frame bytes supplied.
    FrameSizeMismatch {
        /// Size declared in the frame header.
        declared: usize,
        /// Actual number of bytes supplied.
        actual: usize,
    },
    /// The encoded frame would exceed the 9P u32 size field.
    FrameTooLarge {
        /// Encoded frame size in bytes.
        size: usize,
    },
    /// The payload ended before a typed field could be decoded.
    UnexpectedEof {
        /// Number of bytes the decoder needed for the next field.
        needed: usize,
        /// Number of bytes left in the payload.
        remaining: usize,
    },
    /// The payload had bytes left over after typed decoding.
    TrailingPayload {
        /// Number of trailing bytes.
        count: usize,
    },
    /// A 9P string field was not valid UTF-8.
    InvalidUtf8,
    /// A string field cannot fit in the 9P u16 string length.
    StringTooLong {
        /// String length in bytes.
        len: usize,
    },
    /// A byte vector cannot fit in a 9P u32 count field.
    DataTooLong {
        /// Data length in bytes.
        len: usize,
    },
    /// A walk name vector cannot fit in a 9P u16 name count field.
    TooManyWalkNames {
        /// Number of walk names.
        count: usize,
    },
    /// A typed decoder was used with the wrong frame message type.
    UnexpectedMessageType {
        /// Expected 9P message type byte.
        expected: u8,
        /// Actual 9P message type byte.
        actual: u8,
    },
}

impl fmt::Display for P9Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortFrame { len } => write!(f, "9P frame is too short: {len} bytes"),
            Self::InvalidFrameSize { size } => {
                write!(f, "9P frame declares invalid size {size}")
            }
            Self::FrameSizeMismatch { declared, actual } => write!(
                f,
                "9P frame declares {declared} bytes but input has {actual} bytes"
            ),
            Self::FrameTooLarge { size } => {
                write!(f, "9P frame is too large for u32 size field: {size} bytes")
            }
            Self::UnexpectedEof { needed, remaining } => write!(
                f,
                "9P payload needs {needed} bytes but only {remaining} remain"
            ),
            Self::TrailingPayload { count } => {
                write!(f, "9P payload has {count} trailing bytes")
            }
            Self::InvalidUtf8 => f.write_str("9P string is not valid UTF-8"),
            Self::StringTooLong { len } => {
                write!(f, "9P string is too long for u16 length: {len} bytes")
            }
            Self::DataTooLong { len } => {
                write!(f, "9P data is too long for u32 count: {len} bytes")
            }
            Self::TooManyWalkNames { count } => {
                write!(f, "9P walk has too many names for u16 count: {count}")
            }
            Self::UnexpectedMessageType { expected, actual } => {
                write!(f, "9P frame has message type {actual}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for P9Error {}

/// A decoded 9P frame with its typed payload left uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Frame {
    message_type: u8,
    tag: u16,
    payload: Vec<u8>,
}

impl P9Frame {
    /// Creates a frame from a raw message type, tag, and payload.
    #[must_use]
    pub fn new(message_type: u8, tag: u16, payload: Vec<u8>) -> Self {
        Self {
            message_type,
            tag,
            payload,
        }
    }

    /// Returns the raw 9P message type byte.
    #[must_use]
    pub const fn message_type(&self) -> u8 {
        self.message_type
    }

    /// Returns the 9P tag.
    #[must_use]
    pub const fn tag(&self) -> u16 {
        self.tag
    }

    /// Returns the uninterpreted payload bytes after the 9P header.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Decodes one complete 9P frame from exactly one frame of bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame header is too short, declares an
    /// impossible size, or does not match the supplied byte length.
    pub fn decode(bytes: &[u8]) -> Result<Self, P9Error> {
        if bytes.len() < P9_HEADER_LEN {
            return Err(P9Error::ShortFrame { len: bytes.len() });
        }
        let declared = p9_declared_size(bytes)?.expect("header length was checked");
        if declared != bytes.len() {
            return Err(P9Error::FrameSizeMismatch {
                declared,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            message_type: bytes[4],
            tag: p9_tag_from_frame_bytes(bytes)?,
            payload: bytes[P9_HEADER_LEN..].to_vec(),
        })
    }

    /// Encodes this frame into 9P bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame would exceed the 9P u32 size field.
    pub fn encode(&self) -> Result<Vec<u8>, P9Error> {
        let size = P9_HEADER_LEN + self.payload.len();
        let size_u32 = u32::try_from(size).map_err(|_| P9Error::FrameTooLarge { size })?;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&size_u32.to_le_bytes());
        out.push(self.message_type);
        out.extend_from_slice(&self.tag.to_le_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

/// Returns the declared frame size once at least the 4-byte size field exists.
///
/// # Errors
///
/// Returns an error when the declared size is smaller than a 9P header.
pub fn p9_declared_size(bytes: &[u8]) -> Result<Option<usize>, P9Error> {
    if bytes.len() < 4 {
        return Ok(None);
    }
    let size = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("slice length was checked before conversion"),
    ) as usize;
    if size < P9_HEADER_LEN {
        return Err(P9Error::InvalidFrameSize { size });
    }
    Ok(Some(size))
}

/// Reads the 9P tag from frame bytes without decoding the payload.
///
/// # Errors
///
/// Returns an error when fewer than seven bytes are available.
pub fn p9_tag_from_frame_bytes(bytes: &[u8]) -> Result<u16, P9Error> {
    if bytes.len() < P9_HEADER_LEN {
        return Err(P9Error::ShortFrame { len: bytes.len() });
    }
    Ok(u16::from_le_bytes([bytes[5], bytes[6]]))
}

/// Stream accumulator that splits arbitrary byte chunks into complete 9P frames.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct P9FrameBuffer {
    bytes: Vec<u8>,
}

impl P9FrameBuffer {
    /// Creates an empty frame buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of buffered bytes that do not yet form a full frame.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.bytes.len()
    }

    /// Appends bytes and returns every complete 9P frame now available.
    ///
    /// # Errors
    ///
    /// Returns an error when the buffered data declares an impossible frame or
    /// a completed frame fails to decode.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<P9Frame>, P9Error> {
        self.bytes.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(size) = p9_declared_size(&self.bytes)? {
            if self.bytes.len() < size {
                break;
            }
            let frame = P9Frame::decode(&self.bytes[..size])?;
            self.bytes.drain(..size);
            frames.push(frame);
        }
        Ok(frames)
    }
}

/// Decoded payload for `Tversion` and `Rversion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Version {
    /// Maximum 9P message size requested or accepted by the peer.
    pub msize: u32,
    /// Version string, such as `9P2000.L`.
    pub version: String,
}

/// Builds a `Tversion` frame.
///
/// # Errors
///
/// Returns an error when the version string cannot fit in a 9P string field.
pub fn p9_tversion(tag: u16, msize: u32, version: &str) -> Result<P9Frame, P9Error> {
    Ok(P9Frame::new(
        P9_TVERSION,
        tag,
        encode_version_payload(msize, version)?,
    ))
}

/// Builds an `Rversion` frame.
///
/// # Errors
///
/// Returns an error when the version string cannot fit in a 9P string field.
pub fn p9_rversion(tag: u16, msize: u32, version: &str) -> Result<P9Frame, P9Error> {
    Ok(P9Frame::new(
        P9_RVERSION,
        tag,
        encode_version_payload(msize, version)?,
    ))
}

/// Decodes a `Tversion` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tversion` or the payload is
/// malformed.
pub fn p9_decode_tversion(frame: &P9Frame) -> Result<P9Version, P9Error> {
    decode_version_frame(frame, P9_TVERSION)
}

/// Decodes an `Rversion` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rversion` or the payload is
/// malformed.
pub fn p9_decode_rversion(frame: &P9Frame) -> Result<P9Version, P9Error> {
    decode_version_frame(frame, P9_RVERSION)
}

/// 9P QID value used to identify files across a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Qid {
    /// QID type byte.
    pub qid_type: u8,
    /// Server-controlled version value.
    pub version: u32,
    /// Server-controlled path identity.
    pub path: u64,
}

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

/// Decoded payload for `Tauth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Auth {
    /// Fid to attach authentication state to.
    pub afid: u32,
    /// User name string.
    pub uname: String,
    /// Attach name string.
    pub aname: String,
    /// Numeric user id in 9P2000.L auth messages.
    pub n_uname: u32,
}

/// Decoded payload for `Tattach`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Attach {
    /// Fid to attach to the root of the selected tree.
    pub fid: u32,
    /// Auth fid, or [`P9_NOFID`] when unauthenticated.
    pub afid: u32,
    /// User name string.
    pub uname: String,
    /// Attach name string.
    pub aname: String,
    /// Numeric user id in 9P2000.L attach messages.
    pub n_uname: u32,
}

/// Decoded payload for `Twalk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Walk {
    /// Existing fid to walk from.
    pub fid: u32,
    /// Fid to bind to the walked result.
    pub newfid: u32,
    /// Path components to walk.
    pub names: Vec<String>,
}

/// Decoded payload for `Tlopen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Open {
    /// Fid to open.
    pub fid: u32,
    /// Linux open flags carried by 9P2000.L.
    pub flags: u32,
}

/// Decoded payload for `Tlcreate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Create {
    /// Directory fid to create within. The fid becomes the opened file on success.
    pub fid: u32,
    /// Basename to create below the directory fid.
    pub name: String,
    /// Linux open flags carried by 9P2000.L.
    pub flags: u32,
    /// POSIX mode requested for the new file.
    pub mode: u32,
    /// Numeric group id requested for the new file.
    pub gid: u32,
}

/// Decoded payload for `Tsymlink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Symlink {
    /// Directory fid to create the symlink within.
    pub dir_fid: u32,
    /// New symlink basename below `dir_fid`.
    pub name: String,
    /// Uninterpreted symlink target string.
    pub target: String,
    /// Numeric group id requested for the new link.
    pub gid: u32,
}

/// Decoded payload for `Tmknod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Mknod {
    /// Directory fid to create the special file within.
    pub dir_fid: u32,
    /// New node basename below `dir_fid`.
    pub name: String,
    /// POSIX mode requested for the new node.
    pub mode: u32,
    /// Device major number.
    pub major: u32,
    /// Device minor number.
    pub minor: u32,
    /// Numeric group id requested for the new node.
    pub gid: u32,
}

/// Decoded payload for `Treadlink`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9ReadLink {
    /// Fid naming the symlink to read.
    pub fid: u32,
}

/// Decoded payload for `Tgetattr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9GetAttr {
    /// Fid to stat.
    pub fid: u32,
    /// 9P2000.L attribute request mask.
    pub request_mask: u64,
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

/// Decoded payload for `Txattrwalk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9XattrWalk {
    /// Fid naming the file whose extended attribute is being opened.
    pub fid: u32,
    /// Fid to bind to the extended-attribute stream on success.
    pub newfid: u32,
    /// Extended attribute name, or empty string for the xattr name list.
    pub name: String,
}

/// Decoded payload for `Txattrcreate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9XattrCreate {
    /// Fid naming the file whose extended attribute should be created.
    pub fid: u32,
    /// Extended attribute name.
    pub name: String,
    /// Expected extended attribute byte length.
    pub attr_size: u64,
    /// Linux xattr create/replace flags.
    pub flags: u32,
}

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

/// Decoded payload for `Tmkdir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Mkdir {
    /// Directory fid to create within.
    pub dir_fid: u32,
    /// Basename to create below the directory fid.
    pub name: String,
    /// POSIX mode requested for the new directory.
    pub mode: u32,
    /// Numeric group id requested for the new directory.
    pub gid: u32,
}

/// Decoded payload for `Tlink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Link {
    /// Directory fid to create the hard-link name within.
    pub dir_fid: u32,
    /// Existing fid to link to.
    pub fid: u32,
    /// New hard-link basename below `dir_fid`.
    pub name: String,
}

/// Decoded payload for legacy `Trename`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Rename {
    /// Existing fid to move.
    pub fid: u32,
    /// Destination parent directory fid.
    pub dir_fid: u32,
    /// Destination basename below `dir_fid`.
    pub name: String,
}

/// Decoded payload for `Trenameat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9RenameAt {
    /// Source parent directory fid.
    pub old_dir_fid: u32,
    /// Source basename below `old_dir_fid`.
    pub old_name: String,
    /// Destination parent directory fid.
    pub new_dir_fid: u32,
    /// Destination basename below `new_dir_fid`.
    pub new_name: String,
}

/// Decoded payload for `Tunlinkat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9UnlinkAt {
    /// Parent directory fid.
    pub dir_fid: u32,
    /// Basename to remove below `dir_fid`.
    pub name: String,
    /// Linux `unlinkat` flags, including `AT_REMOVEDIR`.
    pub flags: u32,
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

/// Decoded payload for legacy `Tremove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Remove {
    /// Fid to remove and clunk.
    pub fid: u32,
}

/// Builds an `Rlerror` frame.
#[must_use]
pub fn p9_rlerror(tag: u16, ecode: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, ecode);
    P9Frame::new(P9_RLERROR, tag, payload)
}

/// Builds a `Tstatfs` frame.
#[must_use]
pub fn p9_tstatfs(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TSTATFS, tag, payload)
}

/// Builds an `Rstatfs` frame.
#[must_use]
pub fn p9_rstatfs(tag: u16, stat: P9FsStat) -> P9Frame {
    let mut payload = Vec::with_capacity(60);
    push_fs_stat(&mut payload, stat);
    P9Frame::new(P9_RSTATFS, tag, payload)
}

/// Builds a `Tfsync` frame.
#[must_use]
pub fn p9_tfsync(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TFSYNC, tag, payload)
}

/// Builds an `Rfsync` frame.
#[must_use]
pub fn p9_rfsync(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFSYNC, tag, Vec::new())
}

/// Builds a `Tlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_tlock(tag: u16, fid: u32, flags: u32, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    payload.push(lock.lock_type);
    push_u32(&mut payload, flags);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_TLOCK, tag, payload))
}

/// Builds an `Rlock` frame.
#[must_use]
pub fn p9_rlock(tag: u16, status: u8) -> P9Frame {
    P9Frame::new(P9_RLOCK, tag, vec![status])
}

/// Builds a `Tgetlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_tgetlock(tag: u16, fid: u32, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    payload.push(lock.lock_type);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_TGETLOCK, tag, payload))
}

/// Builds an `Rgetlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_rgetlock(tag: u16, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    payload.push(lock.lock_type);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_RGETLOCK, tag, payload))
}

/// Builds a `Tauth` frame.
///
/// # Errors
///
/// Returns an error when a string field cannot fit in a 9P string length.
pub fn p9_tauth(
    tag: u16,
    afid: u32,
    uname: &str,
    aname: &str,
    n_uname: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, afid);
    push_string(&mut payload, uname)?;
    push_string(&mut payload, aname)?;
    push_u32(&mut payload, n_uname);
    Ok(P9Frame::new(P9_TAUTH, tag, payload))
}

/// Builds an `Rauth` frame.
#[must_use]
pub fn p9_rauth(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RAUTH, tag, payload)
}

/// Builds a `Tattach` frame.
///
/// # Errors
///
/// Returns an error when a string field cannot fit in a 9P string length.
pub fn p9_tattach(
    tag: u16,
    fid: u32,
    afid: u32,
    uname: &str,
    aname: &str,
    n_uname: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, afid);
    push_string(&mut payload, uname)?;
    push_string(&mut payload, aname)?;
    push_u32(&mut payload, n_uname);
    Ok(P9Frame::new(P9_TATTACH, tag, payload))
}

/// Builds an `Rattach` frame.
#[must_use]
pub fn p9_rattach(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RATTACH, tag, payload)
}

/// Builds a `Tflush` frame.
#[must_use]
pub fn p9_tflush(tag: u16, oldtag: u16) -> P9Frame {
    let mut payload = Vec::with_capacity(2);
    push_u16(&mut payload, oldtag);
    P9Frame::new(P9_TFLUSH, tag, payload)
}

/// Builds an `Rflush` frame.
#[must_use]
pub fn p9_rflush(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFLUSH, tag, Vec::new())
}

/// Builds a `Twalk` frame.
///
/// # Errors
///
/// Returns an error when too many names are supplied or a name cannot fit in a
/// 9P string length.
pub fn p9_twalk(tag: u16, fid: u32, newfid: u32, names: &[&str]) -> Result<P9Frame, P9Error> {
    let name_count =
        u16::try_from(names.len()).map_err(|_| P9Error::TooManyWalkNames { count: names.len() })?;
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, newfid);
    push_u16(&mut payload, name_count);
    for name in names {
        push_string(&mut payload, name)?;
    }
    Ok(P9Frame::new(P9_TWALK, tag, payload))
}

/// Builds an `Rwalk` frame.
///
/// # Errors
///
/// Returns an error when too many QIDs are supplied.
pub fn p9_rwalk(tag: u16, qids: &[P9Qid]) -> Result<P9Frame, P9Error> {
    let qid_count =
        u16::try_from(qids.len()).map_err(|_| P9Error::TooManyWalkNames { count: qids.len() })?;
    let mut payload = Vec::with_capacity(2 + qids.len() * 13);
    push_u16(&mut payload, qid_count);
    for qid in qids {
        push_qid(&mut payload, *qid);
    }
    Ok(P9Frame::new(P9_RWALK, tag, payload))
}

/// Builds a `Tlopen` frame.
#[must_use]
pub fn p9_tlopen(tag: u16, fid: u32, flags: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(8);
    push_u32(&mut payload, fid);
    push_u32(&mut payload, flags);
    P9Frame::new(P9_TLOPEN, tag, payload)
}

/// Builds an `Rlopen` frame.
#[must_use]
pub fn p9_rlopen(tag: u16, qid: P9Qid, iounit: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(17);
    push_qid(&mut payload, qid);
    push_u32(&mut payload, iounit);
    P9Frame::new(P9_RLOPEN, tag, payload)
}

/// Builds a `Tlcreate` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tlcreate(
    tag: u16,
    fid: u32,
    name: &str,
    flags: u32,
    mode: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, flags);
    push_u32(&mut payload, mode);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TLCREATE, tag, payload))
}

/// Builds an `Rlcreate` frame.
#[must_use]
pub fn p9_rlcreate(tag: u16, qid: P9Qid, iounit: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(17);
    push_qid(&mut payload, qid);
    push_u32(&mut payload, iounit);
    P9Frame::new(P9_RLCREATE, tag, payload)
}

/// Builds a `Tsymlink` frame.
///
/// # Errors
///
/// Returns an error when either string cannot fit in a 9P string field.
pub fn p9_tsymlink(
    tag: u16,
    dir_fid: u32,
    name: &str,
    target: &str,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_string(&mut payload, target)?;
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TSYMLINK, tag, payload))
}

/// Builds an `Rsymlink` frame.
#[must_use]
pub fn p9_rsymlink(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RSYMLINK, tag, payload)
}

/// Builds a `Tmknod` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tmknod(
    tag: u16,
    dir_fid: u32,
    name: &str,
    mode: u32,
    major: u32,
    minor: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, mode);
    push_u32(&mut payload, major);
    push_u32(&mut payload, minor);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TMKNOD, tag, payload))
}

/// Builds an `Rmknod` frame.
#[must_use]
pub fn p9_rmknod(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RMKNOD, tag, payload)
}

/// Builds a `Treadlink` frame.
#[must_use]
pub fn p9_treadlink(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TREADLINK, tag, payload)
}

/// Builds an `Rreadlink` frame.
///
/// # Errors
///
/// Returns an error when the target cannot fit in a 9P string field.
pub fn p9_rreadlink(tag: u16, target: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_string(&mut payload, target)?;
    Ok(P9Frame::new(P9_RREADLINK, tag, payload))
}

/// Builds a `Tgetattr` frame.
#[must_use]
pub fn p9_tgetattr(tag: u16, fid: u32, request_mask: u64) -> P9Frame {
    let mut payload = Vec::with_capacity(12);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, request_mask);
    P9Frame::new(P9_TGETATTR, tag, payload)
}

/// Builds an `Rgetattr` frame.
#[must_use]
pub fn p9_rgetattr(tag: u16, attr: &P9Attr) -> P9Frame {
    let mut payload = Vec::with_capacity(153);
    push_attr(&mut payload, attr);
    P9Frame::new(P9_RGETATTR, tag, payload)
}

/// Builds a `Tsetattr` frame.
#[must_use]
pub fn p9_tsetattr(tag: u16, fid: u32, valid: u32, attr: &P9SetAttr) -> P9Frame {
    let mut payload = Vec::with_capacity(60);
    push_u32(&mut payload, fid);
    push_u32(&mut payload, valid);
    push_set_attr(&mut payload, attr);
    P9Frame::new(P9_TSETATTR, tag, payload)
}

/// Builds an `Rsetattr` frame.
#[must_use]
pub fn p9_rsetattr(tag: u16) -> P9Frame {
    P9Frame::new(P9_RSETATTR, tag, Vec::new())
}

/// Builds a `Txattrwalk` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_txattrwalk(tag: u16, fid: u32, newfid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, newfid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TXATTRWALK, tag, payload))
}

/// Builds an `Rxattrwalk` frame.
#[must_use]
pub fn p9_rxattrwalk(tag: u16, size: u64) -> P9Frame {
    let mut payload = Vec::with_capacity(8);
    push_u64(&mut payload, size);
    P9Frame::new(P9_RXATTRWALK, tag, payload)
}

/// Builds a `Txattrcreate` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_txattrcreate(
    tag: u16,
    fid: u32,
    name: &str,
    attr_size: u64,
    flags: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    push_u64(&mut payload, attr_size);
    push_u32(&mut payload, flags);
    Ok(P9Frame::new(P9_TXATTRCREATE, tag, payload))
}

/// Builds an `Rxattrcreate` frame.
#[must_use]
pub fn p9_rxattrcreate(tag: u16) -> P9Frame {
    P9Frame::new(P9_RXATTRCREATE, tag, Vec::new())
}

/// Builds a `Treaddir` frame.
#[must_use]
pub fn p9_treaddir(tag: u16, fid: u32, offset: u64, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(16);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_u32(&mut payload, count);
    P9Frame::new(P9_TREADDIR, tag, payload)
}

/// Builds an `Rreaddir` frame.
///
/// # Errors
///
/// Returns an error when an entry name cannot fit in a 9P string field or the
/// encoded entry stream cannot fit in a 9P u32 count field.
pub fn p9_rreaddir(tag: u16, entries: &[P9DirEntry]) -> Result<P9Frame, P9Error> {
    let mut data = Vec::new();
    for entry in entries {
        push_dir_entry(&mut data, entry)?;
    }
    let mut payload = Vec::with_capacity(4 + data.len());
    push_counted_data(&mut payload, &data)?;
    Ok(P9Frame::new(P9_RREADDIR, tag, payload))
}

/// Returns the encoded byte length of one 9P2000.L directory entry record.
///
/// # Errors
///
/// Returns an error when the entry name cannot fit in a 9P string field.
pub fn p9_dir_entry_encoded_len(entry: &P9DirEntry) -> Result<usize, P9Error> {
    let name_len = u16::try_from(entry.name.len()).map_err(|_| P9Error::StringTooLong {
        len: entry.name.len(),
    })? as usize;
    Ok(13 + 8 + 1 + 2 + name_len)
}

/// Builds a `Tlink` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tlink(tag: u16, dir_fid: u32, fid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TLINK, tag, payload))
}

/// Builds an `Rlink` frame.
#[must_use]
pub fn p9_rlink(tag: u16) -> P9Frame {
    P9Frame::new(P9_RLINK, tag, Vec::new())
}

/// Builds a legacy `Trename` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_trename(tag: u16, fid: u32, dir_fid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TRENAME, tag, payload))
}

/// Builds a legacy `Rrename` frame.
#[must_use]
pub fn p9_rrename(tag: u16) -> P9Frame {
    P9Frame::new(P9_RRENAME, tag, Vec::new())
}

/// Builds a `Tmkdir` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tmkdir(
    tag: u16,
    dir_fid: u32,
    name: &str,
    mode: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, mode);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TMKDIR, tag, payload))
}

/// Builds an `Rmkdir` frame.
#[must_use]
pub fn p9_rmkdir(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RMKDIR, tag, payload)
}

/// Builds a `Trenameat` frame.
///
/// # Errors
///
/// Returns an error when either name cannot fit in a 9P string field.
pub fn p9_trenameat(
    tag: u16,
    old_dir_fid: u32,
    old_name: &str,
    new_dir_fid: u32,
    new_name: &str,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, old_dir_fid);
    push_string(&mut payload, old_name)?;
    push_u32(&mut payload, new_dir_fid);
    push_string(&mut payload, new_name)?;
    Ok(P9Frame::new(P9_TRENAMEAT, tag, payload))
}

/// Builds an `Rrenameat` frame.
#[must_use]
pub fn p9_rrenameat(tag: u16) -> P9Frame {
    P9Frame::new(P9_RRENAMEAT, tag, Vec::new())
}

/// Builds a `Tunlinkat` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tunlinkat(tag: u16, dir_fid: u32, name: &str, flags: u32) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, flags);
    Ok(P9Frame::new(P9_TUNLINKAT, tag, payload))
}

/// Builds an `Runlinkat` frame.
#[must_use]
pub fn p9_runlinkat(tag: u16) -> P9Frame {
    P9Frame::new(P9_RUNLINKAT, tag, Vec::new())
}

/// Builds a `Tread` frame.
#[must_use]
pub fn p9_tread(tag: u16, fid: u32, offset: u64, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(16);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_u32(&mut payload, count);
    P9Frame::new(P9_TREAD, tag, payload)
}

/// Builds an `Rread` frame.
///
/// # Errors
///
/// Returns an error when the data cannot fit in a 9P u32 count field.
pub fn p9_rread(tag: u16, data: &[u8]) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::with_capacity(4 + data.len());
    push_counted_data(&mut payload, data)?;
    Ok(P9Frame::new(P9_RREAD, tag, payload))
}

/// Builds a `Twrite` frame.
///
/// # Errors
///
/// Returns an error when the data cannot fit in a 9P u32 count field.
pub fn p9_twrite(tag: u16, fid: u32, offset: u64, data: &[u8]) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::with_capacity(16 + data.len());
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_counted_data(&mut payload, data)?;
    Ok(P9Frame::new(P9_TWRITE, tag, payload))
}

/// Builds an `Rwrite` frame.
#[must_use]
pub fn p9_rwrite(tag: u16, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, count);
    P9Frame::new(P9_RWRITE, tag, payload)
}

/// Builds a legacy `Tremove` frame.
#[must_use]
pub fn p9_tremove(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TREMOVE, tag, payload)
}

/// Builds a legacy `Rremove` frame.
#[must_use]
pub fn p9_rremove(tag: u16) -> P9Frame {
    P9Frame::new(P9_RREMOVE, tag, Vec::new())
}

/// Builds a `Tclunk` frame.
#[must_use]
pub fn p9_tclunk(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TCLUNK, tag, payload)
}

/// Builds an `Rclunk` frame.
#[must_use]
pub fn p9_rclunk(tag: u16) -> P9Frame {
    P9Frame::new(P9_RCLUNK, tag, Vec::new())
}

/// Decodes an `Rlerror` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlerror` or the payload is
/// malformed.
pub fn p9_decode_rlerror(frame: &P9Frame) -> Result<P9Lerror, P9Error> {
    expect_message_type(frame, P9_RLERROR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let ecode = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Lerror { ecode })
}

/// Decodes a `Tstatfs` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tstatfs` or the payload is
/// malformed.
pub fn p9_decode_tstatfs(frame: &P9Frame) -> Result<P9StatFs, P9Error> {
    expect_message_type(frame, P9_TSTATFS)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9StatFs { fid })
}

/// Decodes an `Rstatfs` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rstatfs` or the payload is
/// malformed.
pub fn p9_decode_rstatfs(frame: &P9Frame) -> Result<P9FsStat, P9Error> {
    expect_message_type(frame, P9_RSTATFS)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let stat = cursor.read_fs_stat()?;
    cursor.finish()?;
    Ok(stat)
}

/// Decodes a `Tfsync` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tfsync` or the payload is
/// malformed.
pub fn p9_decode_tfsync(frame: &P9Frame) -> Result<P9Fsync, P9Error> {
    expect_message_type(frame, P9_TFSYNC)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Fsync { fid })
}

/// Decodes an `Rfsync` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rfsync` or the payload is not
/// empty.
pub fn p9_decode_rfsync(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RFSYNC)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlock` or the payload is
/// malformed.
pub fn p9_decode_tlock(frame: &P9Frame) -> Result<P9LockRequest, P9Error> {
    expect_message_type(frame, P9_TLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let lock_type = cursor.read_u8()?;
    let flags = cursor.read_u32()?;
    let lock = cursor.read_lock(lock_type)?;
    cursor.finish()?;
    Ok(P9LockRequest { fid, flags, lock })
}

/// Decodes an `Rlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlock` or the payload is
/// malformed.
pub fn p9_decode_rlock(frame: &P9Frame) -> Result<u8, P9Error> {
    expect_message_type(frame, P9_RLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let status = cursor.read_u8()?;
    cursor.finish()?;
    Ok(status)
}

/// Decodes a `Tgetlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tgetlock` or the payload is
/// malformed.
pub fn p9_decode_tgetlock(frame: &P9Frame) -> Result<P9GetLockRequest, P9Error> {
    expect_message_type(frame, P9_TGETLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let lock_type = cursor.read_u8()?;
    let lock = cursor.read_lock(lock_type)?;
    cursor.finish()?;
    Ok(P9GetLockRequest { fid, lock })
}

/// Decodes an `Rgetlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rgetlock` or the payload is
/// malformed.
pub fn p9_decode_rgetlock(frame: &P9Frame) -> Result<P9Lock, P9Error> {
    expect_message_type(frame, P9_RGETLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let lock_type = cursor.read_u8()?;
    let lock = cursor.read_lock(lock_type)?;
    cursor.finish()?;
    Ok(lock)
}

/// Decodes a `Tauth` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tauth` or the payload is
/// malformed.
pub fn p9_decode_tauth(frame: &P9Frame) -> Result<P9Auth, P9Error> {
    expect_message_type(frame, P9_TAUTH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let afid = cursor.read_u32()?;
    let uname = cursor.read_string()?;
    let aname = cursor.read_string()?;
    let n_uname = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Auth {
        afid,
        uname,
        aname,
        n_uname,
    })
}

/// Decodes an `Rauth` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rauth` or the payload is
/// malformed.
pub fn p9_decode_rauth(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RAUTH)
}

/// Decodes a `Tattach` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tattach` or the payload is
/// malformed.
pub fn p9_decode_tattach(frame: &P9Frame) -> Result<P9Attach, P9Error> {
    expect_message_type(frame, P9_TATTACH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let afid = cursor.read_u32()?;
    let uname = cursor.read_string()?;
    let aname = cursor.read_string()?;
    let n_uname = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Attach {
        fid,
        afid,
        uname,
        aname,
        n_uname,
    })
}

/// Decodes an `Rattach` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rattach` or the payload is
/// malformed.
pub fn p9_decode_rattach(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RATTACH)
}

/// Decodes a `Tflush` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tflush` or the payload is
/// malformed.
pub fn p9_decode_tflush(frame: &P9Frame) -> Result<P9Flush, P9Error> {
    expect_message_type(frame, P9_TFLUSH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let oldtag = cursor.read_u16()?;
    cursor.finish()?;
    Ok(P9Flush { oldtag })
}

/// Decodes an `Rflush` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rflush` or the payload is not
/// empty.
pub fn p9_decode_rflush(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RFLUSH)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Twalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Twalk` or the payload is
/// malformed.
pub fn p9_decode_twalk(frame: &P9Frame) -> Result<P9Walk, P9Error> {
    expect_message_type(frame, P9_TWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let newfid = cursor.read_u32()?;
    let name_count = cursor.read_u16()? as usize;
    let mut names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        names.push(cursor.read_string()?);
    }
    cursor.finish()?;
    Ok(P9Walk { fid, newfid, names })
}

/// Decodes an `Rwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rwalk` or the payload is
/// malformed.
pub fn p9_decode_rwalk(frame: &P9Frame) -> Result<Vec<P9Qid>, P9Error> {
    expect_message_type(frame, P9_RWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid_count = cursor.read_u16()? as usize;
    let mut qids = Vec::with_capacity(qid_count);
    for _ in 0..qid_count {
        qids.push(cursor.read_qid()?);
    }
    cursor.finish()?;
    Ok(qids)
}

/// Decodes a `Tlopen` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlopen` or the payload is
/// malformed.
pub fn p9_decode_tlopen(frame: &P9Frame) -> Result<P9Open, P9Error> {
    expect_message_type(frame, P9_TLOPEN)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let flags = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Open { fid, flags })
}

/// Decodes an `Rlopen` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlopen` or the payload is
/// malformed.
pub fn p9_decode_rlopen(frame: &P9Frame) -> Result<(P9Qid, u32), P9Error> {
    expect_message_type(frame, P9_RLOPEN)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    let iounit = cursor.read_u32()?;
    cursor.finish()?;
    Ok((qid, iounit))
}

/// Decodes a `Tlcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlcreate` or the payload is
/// malformed.
pub fn p9_decode_tlcreate(frame: &P9Frame) -> Result<P9Create, P9Error> {
    expect_message_type(frame, P9_TLCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let flags = cursor.read_u32()?;
    let mode = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Create {
        fid,
        name,
        flags,
        mode,
        gid,
    })
}

/// Decodes an `Rlcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlcreate` or the payload is
/// malformed.
pub fn p9_decode_rlcreate(frame: &P9Frame) -> Result<(P9Qid, u32), P9Error> {
    expect_message_type(frame, P9_RLCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    let iounit = cursor.read_u32()?;
    cursor.finish()?;
    Ok((qid, iounit))
}

/// Decodes a `Tsymlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tsymlink` or the payload is
/// malformed.
pub fn p9_decode_tsymlink(frame: &P9Frame) -> Result<P9Symlink, P9Error> {
    expect_message_type(frame, P9_TSYMLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let target = cursor.read_string()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Symlink {
        dir_fid,
        name,
        target,
        gid,
    })
}

/// Decodes an `Rsymlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rsymlink` or the payload is
/// malformed.
pub fn p9_decode_rsymlink(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RSYMLINK)
}

/// Decodes a `Tmknod` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tmknod` or the payload is
/// malformed.
pub fn p9_decode_tmknod(frame: &P9Frame) -> Result<P9Mknod, P9Error> {
    expect_message_type(frame, P9_TMKNOD)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let mode = cursor.read_u32()?;
    let major = cursor.read_u32()?;
    let minor = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Mknod {
        dir_fid,
        name,
        mode,
        major,
        minor,
        gid,
    })
}

/// Decodes an `Rmknod` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rmknod` or the payload is
/// malformed.
pub fn p9_decode_rmknod(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RMKNOD)
}

/// Decodes a `Treadlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Treadlink` or the payload is
/// malformed.
pub fn p9_decode_treadlink(frame: &P9Frame) -> Result<P9ReadLink, P9Error> {
    expect_message_type(frame, P9_TREADLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9ReadLink { fid })
}

/// Decodes an `Rreadlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rreadlink` or the payload is
/// malformed.
pub fn p9_decode_rreadlink(frame: &P9Frame) -> Result<String, P9Error> {
    expect_message_type(frame, P9_RREADLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let target = cursor.read_string()?;
    cursor.finish()?;
    Ok(target)
}

/// Decodes a `Tgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tgetattr` or the payload is
/// malformed.
pub fn p9_decode_tgetattr(frame: &P9Frame) -> Result<P9GetAttr, P9Error> {
    expect_message_type(frame, P9_TGETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let request_mask = cursor.read_u64()?;
    cursor.finish()?;
    Ok(P9GetAttr { fid, request_mask })
}

/// Decodes an `Rgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rgetattr` or the payload is
/// malformed.
pub fn p9_decode_rgetattr(frame: &P9Frame) -> Result<P9Attr, P9Error> {
    expect_message_type(frame, P9_RGETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let attr = cursor.read_attr()?;
    cursor.finish()?;
    Ok(attr)
}

/// Decodes a `Tsetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tsetattr` or the payload is
/// malformed.
pub fn p9_decode_tsetattr(frame: &P9Frame) -> Result<P9SetAttrRequest, P9Error> {
    expect_message_type(frame, P9_TSETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let valid = cursor.read_u32()?;
    let attr = cursor.read_set_attr()?;
    cursor.finish()?;
    Ok(P9SetAttrRequest { fid, valid, attr })
}

/// Decodes an `Rsetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rsetattr` or the payload is not
/// empty.
pub fn p9_decode_rsetattr(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RSETATTR)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Txattrwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Txattrwalk` or the payload is
/// malformed.
pub fn p9_decode_txattrwalk(frame: &P9Frame) -> Result<P9XattrWalk, P9Error> {
    expect_message_type(frame, P9_TXATTRWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let newfid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9XattrWalk { fid, newfid, name })
}

/// Decodes an `Rxattrwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rxattrwalk` or the payload is
/// malformed.
pub fn p9_decode_rxattrwalk(frame: &P9Frame) -> Result<u64, P9Error> {
    expect_message_type(frame, P9_RXATTRWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let size = cursor.read_u64()?;
    cursor.finish()?;
    Ok(size)
}

/// Decodes a `Txattrcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Txattrcreate` or the payload is
/// malformed.
pub fn p9_decode_txattrcreate(frame: &P9Frame) -> Result<P9XattrCreate, P9Error> {
    expect_message_type(frame, P9_TXATTRCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let attr_size = cursor.read_u64()?;
    let flags = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9XattrCreate {
        fid,
        name,
        attr_size,
        flags,
    })
}

/// Decodes an `Rxattrcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rxattrcreate` or the payload is
/// not empty.
pub fn p9_decode_rxattrcreate(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RXATTRCREATE)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Treaddir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Treaddir` or the payload is
/// malformed.
pub fn p9_decode_treaddir(frame: &P9Frame) -> Result<P9ReadDir, P9Error> {
    expect_message_type(frame, P9_TREADDIR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let count = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9ReadDir { fid, offset, count })
}

/// Decodes an `Rreaddir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rreaddir` or the payload is
/// malformed.
pub fn p9_decode_rreaddir(frame: &P9Frame) -> Result<Vec<P9DirEntry>, P9Error> {
    expect_message_type(frame, P9_RREADDIR)?;
    let data = decode_data_frame(frame, P9_RREADDIR)?;
    let mut cursor = PayloadCursor::new(&data);
    let mut entries = Vec::new();
    while cursor.remaining_len() > 0 {
        entries.push(cursor.read_dir_entry()?);
    }
    Ok(entries)
}

/// Decodes a `Tlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlink` or the payload is
/// malformed.
pub fn p9_decode_tlink(frame: &P9Frame) -> Result<P9Link, P9Error> {
    expect_message_type(frame, P9_TLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9Link { dir_fid, fid, name })
}

/// Decodes an `Rlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlink` or the payload is not
/// empty.
pub fn p9_decode_rlink(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RLINK)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a legacy `Trename` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Trename` or the payload is
/// malformed.
pub fn p9_decode_trename(frame: &P9Frame) -> Result<P9Rename, P9Error> {
    expect_message_type(frame, P9_TRENAME)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9Rename { fid, dir_fid, name })
}

/// Decodes a legacy `Rrename` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rrename` or the payload is
/// not empty.
pub fn p9_decode_rrename(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RRENAME)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tmkdir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tmkdir` or the payload is
/// malformed.
pub fn p9_decode_tmkdir(frame: &P9Frame) -> Result<P9Mkdir, P9Error> {
    expect_message_type(frame, P9_TMKDIR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let mode = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Mkdir {
        dir_fid,
        name,
        mode,
        gid,
    })
}

/// Decodes an `Rmkdir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rmkdir` or the payload is
/// malformed.
pub fn p9_decode_rmkdir(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RMKDIR)
}

/// Decodes a `Trenameat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Trenameat` or the payload is
/// malformed.
pub fn p9_decode_trenameat(frame: &P9Frame) -> Result<P9RenameAt, P9Error> {
    expect_message_type(frame, P9_TRENAMEAT)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let old_dir_fid = cursor.read_u32()?;
    let old_name = cursor.read_string()?;
    let new_dir_fid = cursor.read_u32()?;
    let new_name = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9RenameAt {
        old_dir_fid,
        old_name,
        new_dir_fid,
        new_name,
    })
}

/// Decodes an `Rrenameat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rrenameat` or the payload is
/// not empty.
pub fn p9_decode_rrenameat(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RRENAMEAT)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tunlinkat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tunlinkat` or the payload is
/// malformed.
pub fn p9_decode_tunlinkat(frame: &P9Frame) -> Result<P9UnlinkAt, P9Error> {
    expect_message_type(frame, P9_TUNLINKAT)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let flags = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9UnlinkAt {
        dir_fid,
        name,
        flags,
    })
}

/// Decodes an `Runlinkat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Runlinkat` or the payload is
/// not empty.
pub fn p9_decode_runlinkat(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RUNLINKAT)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tread` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tread` or the payload is
/// malformed.
pub fn p9_decode_tread(frame: &P9Frame) -> Result<P9Read, P9Error> {
    expect_message_type(frame, P9_TREAD)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let count = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Read { fid, offset, count })
}

/// Decodes an `Rread` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rread` or the payload is
/// malformed.
pub fn p9_decode_rread(frame: &P9Frame) -> Result<Vec<u8>, P9Error> {
    decode_data_frame(frame, P9_RREAD)
}

/// Decodes a `Twrite` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Twrite` or the payload is
/// malformed.
pub fn p9_decode_twrite(frame: &P9Frame) -> Result<P9Write, P9Error> {
    expect_message_type(frame, P9_TWRITE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let data = cursor.read_counted_data()?;
    cursor.finish()?;
    Ok(P9Write { fid, offset, data })
}

/// Decodes an `Rwrite` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rwrite` or the payload is
/// malformed.
pub fn p9_decode_rwrite(frame: &P9Frame) -> Result<u32, P9Error> {
    expect_message_type(frame, P9_RWRITE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let count = cursor.read_u32()?;
    cursor.finish()?;
    Ok(count)
}

/// Decodes a legacy `Tremove` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tremove` or the payload is
/// malformed.
pub fn p9_decode_tremove(frame: &P9Frame) -> Result<P9Remove, P9Error> {
    expect_message_type(frame, P9_TREMOVE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Remove { fid })
}

/// Decodes a legacy `Rremove` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rremove` or the payload is
/// not empty.
pub fn p9_decode_rremove(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RREMOVE)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tclunk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tclunk` or the payload is
/// malformed.
pub fn p9_decode_tclunk(frame: &P9Frame) -> Result<P9Clunk, P9Error> {
    expect_message_type(frame, P9_TCLUNK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Clunk { fid })
}

/// Decodes an `Rclunk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rclunk` or the payload is not
/// empty.
pub fn p9_decode_rclunk(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RCLUNK)?;
    PayloadCursor::new(frame.payload()).finish()
}

fn encode_version_payload(msize: u32, version: &str) -> Result<Vec<u8>, P9Error> {
    let version_bytes = version.as_bytes();
    let version_len = u16::try_from(version_bytes.len()).map_err(|_| P9Error::StringTooLong {
        len: version_bytes.len(),
    })?;
    let mut payload = Vec::with_capacity(6 + version_bytes.len());
    payload.extend_from_slice(&msize.to_le_bytes());
    payload.extend_from_slice(&version_len.to_le_bytes());
    payload.extend_from_slice(version_bytes);
    Ok(payload)
}

fn decode_version_frame(frame: &P9Frame, expected: u8) -> Result<P9Version, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(&frame.payload);
    let msize = cursor.read_u32()?;
    let version = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9Version { msize, version })
}

fn decode_qid_frame(frame: &P9Frame, expected: u8) -> Result<P9Qid, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    cursor.finish()?;
    Ok(qid)
}

fn decode_data_frame(frame: &P9Frame, expected: u8) -> Result<Vec<u8>, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let data = cursor.read_counted_data()?;
    cursor.finish()?;
    Ok(data)
}

fn expect_message_type(frame: &P9Frame, expected: u8) -> Result<(), P9Error> {
    if frame.message_type == expected {
        Ok(())
    } else {
        Err(P9Error::UnexpectedMessageType {
            expected,
            actual: frame.message_type,
        })
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), P9Error> {
    let bytes = value.as_bytes();
    let len =
        u16::try_from(bytes.len()).map_err(|_| P9Error::StringTooLong { len: bytes.len() })?;
    push_u16(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}

fn push_qid(out: &mut Vec<u8>, qid: P9Qid) {
    out.push(qid.qid_type);
    push_u32(out, qid.version);
    push_u64(out, qid.path);
}

fn push_fs_stat(out: &mut Vec<u8>, stat: P9FsStat) {
    push_u32(out, stat.fs_type);
    push_u32(out, stat.block_size);
    push_u64(out, stat.blocks);
    push_u64(out, stat.blocks_free);
    push_u64(out, stat.blocks_available);
    push_u64(out, stat.files);
    push_u64(out, stat.files_free);
    push_u64(out, stat.fsid);
    push_u32(out, stat.name_length);
}

fn push_attr(out: &mut Vec<u8>, attr: &P9Attr) {
    push_u64(out, attr.valid);
    push_qid(out, attr.qid);
    push_u32(out, attr.mode);
    push_u32(out, attr.uid);
    push_u32(out, attr.gid);
    push_u64(out, attr.nlink);
    push_u64(out, attr.rdev);
    push_u64(out, attr.size);
    push_u64(out, attr.block_size);
    push_u64(out, attr.blocks);
    push_u64(out, attr.atime_seconds);
    push_u64(out, attr.atime_nanoseconds);
    push_u64(out, attr.mtime_seconds);
    push_u64(out, attr.mtime_nanoseconds);
    push_u64(out, attr.ctime_seconds);
    push_u64(out, attr.ctime_nanoseconds);
    push_u64(out, attr.btime_seconds);
    push_u64(out, attr.btime_nanoseconds);
    push_u64(out, attr.generation);
    push_u64(out, attr.data_version);
}

fn push_set_attr(out: &mut Vec<u8>, attr: &P9SetAttr) {
    push_u32(out, attr.permissions);
    push_u32(out, attr.uid);
    push_u32(out, attr.gid);
    push_u64(out, attr.size);
    push_u64(out, attr.atime_seconds);
    push_u64(out, attr.atime_nanoseconds);
    push_u64(out, attr.mtime_seconds);
    push_u64(out, attr.mtime_nanoseconds);
}

fn push_lock_range(out: &mut Vec<u8>, lock: &P9Lock) -> Result<(), P9Error> {
    push_u64(out, lock.start);
    push_u64(out, lock.length);
    push_u32(out, lock.proc_id);
    push_string(out, &lock.client_id)?;
    Ok(())
}

fn push_dir_entry(out: &mut Vec<u8>, entry: &P9DirEntry) -> Result<(), P9Error> {
    push_qid(out, entry.qid);
    push_u64(out, entry.offset);
    out.push(entry.dirent_type);
    push_string(out, &entry.name)?;
    Ok(())
}

fn push_counted_data(out: &mut Vec<u8>, data: &[u8]) -> Result<(), P9Error> {
    let len = u32::try_from(data.len()).map_err(|_| P9Error::DataTooLong { len: data.len() })?;
    push_u32(out, len);
    out.extend_from_slice(data);
    Ok(())
}

struct PayloadCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PayloadCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_u32(&mut self) -> Result<u32, P9Error> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    fn read_u16(&mut self) -> Result<u16, P9Error> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    fn read_u8(&mut self) -> Result<u8, P9Error> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_u64(&mut self) -> Result<u64, P9Error> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    fn read_qid(&mut self) -> Result<P9Qid, P9Error> {
        let qid_type = self.read_exact(1)?[0];
        let version = self.read_u32()?;
        let path = self.read_u64()?;
        Ok(P9Qid {
            qid_type,
            version,
            path,
        })
    }

    fn read_fs_stat(&mut self) -> Result<P9FsStat, P9Error> {
        let fs_type = self.read_u32()?;
        let block_size = self.read_u32()?;
        let blocks = self.read_u64()?;
        let blocks_free = self.read_u64()?;
        let blocks_available = self.read_u64()?;
        let files = self.read_u64()?;
        let files_free = self.read_u64()?;
        let fsid = self.read_u64()?;
        let name_length = self.read_u32()?;
        Ok(P9FsStat {
            fs_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
            fsid,
            name_length,
        })
    }

    fn read_dir_entry(&mut self) -> Result<P9DirEntry, P9Error> {
        let qid = self.read_qid()?;
        let offset = self.read_u64()?;
        let dirent_type = self.read_u8()?;
        let name = self.read_string()?;
        Ok(P9DirEntry {
            qid,
            offset,
            dirent_type,
            name,
        })
    }

    fn read_attr(&mut self) -> Result<P9Attr, P9Error> {
        let valid = self.read_u64()?;
        let qid = self.read_qid()?;
        let mode = self.read_u32()?;
        let uid = self.read_u32()?;
        let gid = self.read_u32()?;
        let nlink = self.read_u64()?;
        let rdev = self.read_u64()?;
        let size = self.read_u64()?;
        let block_size = self.read_u64()?;
        let blocks = self.read_u64()?;
        let atime_seconds = self.read_u64()?;
        let atime_nanoseconds = self.read_u64()?;
        let mtime_seconds = self.read_u64()?;
        let mtime_nanoseconds = self.read_u64()?;
        let ctime_seconds = self.read_u64()?;
        let ctime_nanoseconds = self.read_u64()?;
        let btime_seconds = self.read_u64()?;
        let btime_nanoseconds = self.read_u64()?;
        let generation = self.read_u64()?;
        let data_version = self.read_u64()?;
        Ok(P9Attr {
            valid,
            qid,
            mode,
            uid,
            gid,
            nlink,
            rdev,
            size,
            block_size,
            blocks,
            atime_seconds,
            atime_nanoseconds,
            mtime_seconds,
            mtime_nanoseconds,
            ctime_seconds,
            ctime_nanoseconds,
            btime_seconds,
            btime_nanoseconds,
            generation,
            data_version,
        })
    }

    fn read_set_attr(&mut self) -> Result<P9SetAttr, P9Error> {
        let permissions = self.read_u32()?;
        let uid = self.read_u32()?;
        let gid = self.read_u32()?;
        let size = self.read_u64()?;
        let atime_seconds = self.read_u64()?;
        let atime_nanoseconds = self.read_u64()?;
        let mtime_seconds = self.read_u64()?;
        let mtime_nanoseconds = self.read_u64()?;
        Ok(P9SetAttr {
            permissions,
            uid,
            gid,
            size,
            atime_seconds,
            atime_nanoseconds,
            mtime_seconds,
            mtime_nanoseconds,
        })
    }

    fn read_lock(&mut self, lock_type: u8) -> Result<P9Lock, P9Error> {
        let start = self.read_u64()?;
        let length = self.read_u64()?;
        let proc_id = self.read_u32()?;
        let client_id = self.read_string()?;
        Ok(P9Lock {
            lock_type,
            start,
            length,
            proc_id,
            client_id,
        })
    }

    fn read_string(&mut self) -> Result<String, P9Error> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| P9Error::InvalidUtf8)
    }

    fn read_counted_data(&mut self) -> Result<Vec<u8>, P9Error> {
        let len = self.read_u32()? as usize;
        Ok(self.read_exact(len)?.to_vec())
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], P9Error> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if remaining < len {
            return Err(P9Error::UnexpectedEof {
                needed: len,
                remaining,
            });
        }
        let start = self.offset;
        self.offset += len;
        Ok(&self.bytes[start..start + len])
    }

    fn finish(self) -> Result<(), P9Error> {
        let count = self.bytes.len().saturating_sub(self.offset);
        if count == 0 {
            Ok(())
        } else {
            Err(P9Error::TrailingPayload { count })
        }
    }

    fn remaining_len(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_is_declared() {
        assert!(!crate::CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn version_frame_round_trips_9p2000_l() {
        let frame = p9_tversion(P9_NOTAG, 131_072, P9_VERSION_9P2000_L).unwrap();

        assert_eq!(frame.message_type(), P9_TVERSION);
        assert_eq!(frame.tag(), P9_NOTAG);

        let bytes = frame.encode().unwrap();
        assert_eq!(&bytes[..4], &21_u32.to_le_bytes());
        assert_eq!(bytes[4], P9_TVERSION);
        assert_eq!(p9_tag_from_frame_bytes(&bytes).unwrap(), P9_NOTAG);

        let decoded = P9Frame::decode(&bytes).unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(
            p9_decode_tversion(&decoded).unwrap(),
            P9Version {
                msize: 131_072,
                version: P9_VERSION_9P2000_L.to_owned()
            }
        );
    }

    #[test]
    fn rversion_uses_same_payload_shape_as_tversion() {
        let frame = p9_rversion(7, 65_536, "9P2000").unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(
            p9_decode_rversion(&decoded).unwrap(),
            P9Version {
                msize: 65_536,
                version: "9P2000".to_owned()
            }
        );
        assert_eq!(
            p9_decode_tversion(&decoded).unwrap_err(),
            P9Error::UnexpectedMessageType {
                expected: P9_TVERSION,
                actual: P9_RVERSION
            }
        );
    }

    #[test]
    fn frame_decode_accepts_unknown_message_types() {
        let frame = P9Frame::new(250, 9, vec![1, 2, 3]);
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(decoded.message_type(), 250);
        assert_eq!(decoded.tag(), 9);
        assert_eq!(decoded.payload(), &[1, 2, 3]);
        assert_eq!(p9_message_type_name(250), None);
    }

    #[test]
    fn frame_decode_rejects_invalid_sizes() {
        assert_eq!(
            P9Frame::decode(&[1, 2, 3]).unwrap_err(),
            P9Error::ShortFrame { len: 3 }
        );

        let mut too_small = Vec::new();
        too_small.extend_from_slice(&6_u32.to_le_bytes());
        too_small.extend_from_slice(&[P9_TVERSION, 1, 0]);
        assert_eq!(
            P9Frame::decode(&too_small).unwrap_err(),
            P9Error::InvalidFrameSize { size: 6 }
        );

        let mut mismatch = Vec::new();
        mismatch.extend_from_slice(&8_u32.to_le_bytes());
        mismatch.extend_from_slice(&[P9_TVERSION, 1, 0]);
        assert_eq!(
            P9Frame::decode(&mismatch).unwrap_err(),
            P9Error::FrameSizeMismatch {
                declared: 8,
                actual: 7
            }
        );
    }

    #[test]
    fn frame_buffer_splits_partial_and_multiple_frames() {
        let first = p9_tversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        let second = p9_rversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        let mut buffer = P9FrameBuffer::new();

        assert!(buffer.push(&first[..3]).unwrap().is_empty());
        assert_eq!(buffer.buffered_len(), 3);

        let frames = buffer.push(&first[3..]).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].message_type(), P9_TVERSION);
        assert_eq!(buffer.buffered_len(), 0);

        let mut combined = Vec::new();
        combined.extend_from_slice(&first);
        combined.extend_from_slice(&second);
        let frames = buffer.push(&combined).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].message_type(), P9_TVERSION);
        assert_eq!(frames[1].message_type(), P9_RVERSION);
    }

    #[test]
    fn version_payload_rejects_malformed_strings_and_trailing_bytes() {
        let truncated = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 4, 0, b'9']);
        assert_eq!(
            p9_decode_tversion(&truncated).unwrap_err(),
            P9Error::UnexpectedEof {
                needed: 4,
                remaining: 1
            }
        );

        let invalid_utf8 = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 1, 0, 0xff]);
        assert_eq!(
            p9_decode_tversion(&invalid_utf8).unwrap_err(),
            P9Error::InvalidUtf8
        );

        let trailing = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 0, 0, 99]);
        assert_eq!(
            p9_decode_tversion(&trailing).unwrap_err(),
            P9Error::TrailingPayload { count: 1 }
        );
    }

    #[test]
    fn core_message_type_names_are_known() {
        assert_eq!(p9_message_type_name(P9_RLERROR), Some("Rlerror"));
        assert_eq!(p9_message_type_name(P9_TSTATFS), Some("Tstatfs"));
        assert_eq!(p9_message_type_name(P9_RSTATFS), Some("Rstatfs"));
        assert_eq!(p9_message_type_name(P9_TLOPEN), Some("Tlopen"));
        assert_eq!(p9_message_type_name(P9_RLOPEN), Some("Rlopen"));
        assert_eq!(p9_message_type_name(P9_TLCREATE), Some("Tlcreate"));
        assert_eq!(p9_message_type_name(P9_RLCREATE), Some("Rlcreate"));
        assert_eq!(p9_message_type_name(P9_TSYMLINK), Some("Tsymlink"));
        assert_eq!(p9_message_type_name(P9_RSYMLINK), Some("Rsymlink"));
        assert_eq!(p9_message_type_name(P9_TMKNOD), Some("Tmknod"));
        assert_eq!(p9_message_type_name(P9_RMKNOD), Some("Rmknod"));
        assert_eq!(p9_message_type_name(P9_TRENAME), Some("Trename"));
        assert_eq!(p9_message_type_name(P9_RRENAME), Some("Rrename"));
        assert_eq!(p9_message_type_name(P9_TREADLINK), Some("Treadlink"));
        assert_eq!(p9_message_type_name(P9_RREADLINK), Some("Rreadlink"));
        assert_eq!(p9_message_type_name(P9_TGETATTR), Some("Tgetattr"));
        assert_eq!(p9_message_type_name(P9_RGETATTR), Some("Rgetattr"));
        assert_eq!(p9_message_type_name(P9_TSETATTR), Some("Tsetattr"));
        assert_eq!(p9_message_type_name(P9_RSETATTR), Some("Rsetattr"));
        assert_eq!(p9_message_type_name(P9_TXATTRWALK), Some("Txattrwalk"));
        assert_eq!(p9_message_type_name(P9_RXATTRWALK), Some("Rxattrwalk"));
        assert_eq!(p9_message_type_name(P9_TXATTRCREATE), Some("Txattrcreate"));
        assert_eq!(p9_message_type_name(P9_RXATTRCREATE), Some("Rxattrcreate"));
        assert_eq!(p9_message_type_name(P9_TREADDIR), Some("Treaddir"));
        assert_eq!(p9_message_type_name(P9_RREADDIR), Some("Rreaddir"));
        assert_eq!(p9_message_type_name(P9_TFSYNC), Some("Tfsync"));
        assert_eq!(p9_message_type_name(P9_RFSYNC), Some("Rfsync"));
        assert_eq!(p9_message_type_name(P9_TLOCK), Some("Tlock"));
        assert_eq!(p9_message_type_name(P9_RLOCK), Some("Rlock"));
        assert_eq!(p9_message_type_name(P9_TGETLOCK), Some("Tgetlock"));
        assert_eq!(p9_message_type_name(P9_RGETLOCK), Some("Rgetlock"));
        assert_eq!(p9_message_type_name(P9_TLINK), Some("Tlink"));
        assert_eq!(p9_message_type_name(P9_RLINK), Some("Rlink"));
        assert_eq!(p9_message_type_name(P9_TMKDIR), Some("Tmkdir"));
        assert_eq!(p9_message_type_name(P9_RMKDIR), Some("Rmkdir"));
        assert_eq!(p9_message_type_name(P9_TRENAMEAT), Some("Trenameat"));
        assert_eq!(p9_message_type_name(P9_RRENAMEAT), Some("Rrenameat"));
        assert_eq!(p9_message_type_name(P9_TUNLINKAT), Some("Tunlinkat"));
        assert_eq!(p9_message_type_name(P9_RUNLINKAT), Some("Runlinkat"));
        assert_eq!(p9_message_type_name(P9_TAUTH), Some("Tauth"));
        assert_eq!(p9_message_type_name(P9_RAUTH), Some("Rauth"));
        assert_eq!(p9_message_type_name(P9_TATTACH), Some("Tattach"));
        assert_eq!(p9_message_type_name(P9_RATTACH), Some("Rattach"));
        assert_eq!(p9_message_type_name(P9_TFLUSH), Some("Tflush"));
        assert_eq!(p9_message_type_name(P9_RFLUSH), Some("Rflush"));
        assert_eq!(p9_message_type_name(P9_TWALK), Some("Twalk"));
        assert_eq!(p9_message_type_name(P9_RWALK), Some("Rwalk"));
        assert_eq!(p9_message_type_name(P9_TREAD), Some("Tread"));
        assert_eq!(p9_message_type_name(P9_RREAD), Some("Rread"));
        assert_eq!(p9_message_type_name(P9_TWRITE), Some("Twrite"));
        assert_eq!(p9_message_type_name(P9_RWRITE), Some("Rwrite"));
        assert_eq!(p9_message_type_name(P9_TCLUNK), Some("Tclunk"));
        assert_eq!(p9_message_type_name(P9_RCLUNK), Some("Rclunk"));
        assert_eq!(p9_message_type_name(P9_TREMOVE), Some("Tremove"));
        assert_eq!(p9_message_type_name(P9_RREMOVE), Some("Rremove"));
    }

    #[test]
    fn lerror_round_trips_linux_errno() {
        let frame = p9_rlerror(3, 2);
        let bytes = frame.encode().unwrap();

        assert_eq!(&bytes[..4], &11_u32.to_le_bytes());
        assert_eq!(bytes[4], P9_RLERROR);
        assert_eq!(
            p9_decode_rlerror(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Lerror { ecode: 2 }
        );
    }

    #[test]
    fn statfs_round_trips_9p2000_l_payload() {
        let frame = p9_tstatfs(4, 10).encode().unwrap();
        assert_eq!(&frame[..4], &11_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TSTATFS);
        assert_eq!(
            p9_decode_tstatfs(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9StatFs { fid: 10 }
        );

        let stat = P9FsStat {
            fs_type: 0x0102_1997,
            block_size: 4096,
            blocks: 1,
            blocks_free: 2,
            blocks_available: 3,
            files: 4,
            files_free: 5,
            fsid: 6,
            name_length: 255,
        };
        let response = p9_rstatfs(4, stat).encode().unwrap();
        assert_eq!(&response[..4], &67_u32.to_le_bytes());
        assert_eq!(response[4], P9_RSTATFS);
        assert_eq!(
            p9_decode_rstatfs(&P9Frame::decode(&response).unwrap()).unwrap(),
            stat
        );
    }

    #[test]
    fn fsync_round_trips_empty_response() {
        let frame = p9_tfsync(4, 10).encode().unwrap();
        assert_eq!(&frame[..4], &11_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TFSYNC);
        assert_eq!(
            p9_decode_tfsync(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Fsync { fid: 10 }
        );

        let response = p9_rfsync(4).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RFSYNC);
        p9_decode_rfsync(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn lock_round_trips_status_response() {
        let lock = P9Lock {
            lock_type: P9_LOCK_TYPE_WRITE,
            start: 11,
            length: 22,
            proc_id: 33,
            client_id: "linux-node".to_owned(),
        };
        let frame = p9_tlock(5, 10, 1, &lock).unwrap().encode().unwrap();
        assert_eq!(&frame[..4], &48_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TLOCK);
        assert_eq!(
            p9_decode_tlock(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9LockRequest {
                fid: 10,
                flags: 1,
                lock
            }
        );

        let response = p9_rlock(5, P9_LOCK_STATUS_OK).encode().unwrap();
        assert_eq!(&response[..4], &8_u32.to_le_bytes());
        assert_eq!(response[4], P9_RLOCK);
        assert_eq!(
            p9_decode_rlock(&P9Frame::decode(&response).unwrap()).unwrap(),
            P9_LOCK_STATUS_OK
        );
    }

    #[test]
    fn getlock_round_trips_no_conflict_payload() {
        let request_lock = P9Lock {
            lock_type: P9_LOCK_TYPE_READ,
            start: 44,
            length: 55,
            proc_id: 66,
            client_id: "client-a".to_owned(),
        };
        let frame = p9_tgetlock(6, 11, &request_lock).unwrap().encode().unwrap();
        assert_eq!(&frame[..4], &42_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TGETLOCK);
        assert_eq!(
            p9_decode_tgetlock(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9GetLockRequest {
                fid: 11,
                lock: request_lock
            }
        );

        let response_lock = P9Lock {
            lock_type: P9_LOCK_TYPE_UNLOCK,
            start: 44,
            length: 55,
            proc_id: 0,
            client_id: String::new(),
        };
        let response = p9_rgetlock(6, &response_lock).unwrap().encode().unwrap();
        assert_eq!(&response[..4], &30_u32.to_le_bytes());
        assert_eq!(response[4], P9_RGETLOCK);
        assert_eq!(
            p9_decode_rgetlock(&P9Frame::decode(&response).unwrap()).unwrap(),
            response_lock
        );
    }

    #[test]
    fn auth_round_trips_9p2000_l_payload() {
        let frame = p9_tauth(4, 10, "root", "main", 1000).unwrap();
        let bytes = frame.encode().unwrap();

        assert_eq!(bytes[4], P9_TAUTH);
        assert_eq!(
            p9_decode_tauth(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Auth {
                afid: 10,
                uname: "root".to_owned(),
                aname: "main".to_owned(),
                n_uname: 1000
            }
        );

        let qid = qid(0, 0x0102_0304, 0x0506_0708_090a_0b0c);
        let response = p9_rauth(4, qid);
        assert_eq!(
            p9_decode_rauth(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn attach_round_trips_9p2000_l_payload() {
        let frame = p9_tattach(4, 10, P9_NOFID, "root", "", 1000).unwrap();
        let bytes = frame.encode().unwrap();

        assert_eq!(bytes[4], P9_TATTACH);
        assert_eq!(
            p9_decode_tattach(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Attach {
                fid: 10,
                afid: P9_NOFID,
                uname: "root".to_owned(),
                aname: String::new(),
                n_uname: 1000
            }
        );

        let qid = qid(0x80, 0x0102_0304, 0x0506_0708_090a_0b0c);
        let response = p9_rattach(4, qid);
        assert_eq!(
            p9_decode_rattach(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn flush_round_trips_oldtag_and_empty_response() {
        let frame = p9_tflush(6, 42).encode().unwrap();
        assert_eq!(&frame[..4], &9_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TFLUSH);
        assert_eq!(
            p9_decode_tflush(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Flush { oldtag: 42 }
        );

        let response = p9_rflush(6).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RFLUSH);
        p9_decode_rflush(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn walk_round_trips_names_and_qids() {
        let frame = p9_twalk(5, 10, 11, &["bin", "sh"]).unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(
            p9_decode_twalk(&decoded).unwrap(),
            P9Walk {
                fid: 10,
                newfid: 11,
                names: vec!["bin".to_owned(), "sh".to_owned()]
            }
        );

        let first = qid(0x80, 0, 1);
        let second = qid(0, 0, 2);
        let response = p9_rwalk(5, &[first, second]).unwrap();
        assert_eq!(
            p9_decode_rwalk(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn lopen_round_trips_linux_flags_and_iounit() {
        let frame = p9_tlopen(6, 22, 0x8000).encode().unwrap();
        assert_eq!(
            p9_decode_tlopen(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Open {
                fid: 22,
                flags: 0x8000
            }
        );

        let qid = qid(0, 7, 9);
        let response = p9_rlopen(6, qid, 8192).encode().unwrap();
        assert_eq!(
            p9_decode_rlopen(&P9Frame::decode(&response).unwrap()).unwrap(),
            (qid, 8192)
        );
    }

    #[test]
    fn lcreate_round_trips_name_flags_mode_gid_and_iounit() {
        let frame = p9_tlcreate(12, 22, "new.txt", 0x241, 0o100664, 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&frame[..4], &32_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tlcreate(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Create {
                fid: 22,
                name: "new.txt".to_owned(),
                flags: 0x241,
                mode: 0o100664,
                gid: 1000
            }
        );

        let qid = qid(0, 7, 9);
        let response = p9_rlcreate(12, qid, 8192).encode().unwrap();
        assert_eq!(&response[..4], &24_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rlcreate(&P9Frame::decode(&response).unwrap()).unwrap(),
            (qid, 8192)
        );
    }

    #[test]
    fn symlink_and_readlink_round_trip() {
        let symlink = p9_tsymlink(16, 1, "link.txt", "target.txt", 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&symlink[..4], &37_u32.to_le_bytes());
        assert_eq!(symlink[4], P9_TSYMLINK);
        assert_eq!(
            p9_decode_tsymlink(&P9Frame::decode(&symlink).unwrap()).unwrap(),
            P9Symlink {
                dir_fid: 1,
                name: "link.txt".to_owned(),
                target: "target.txt".to_owned(),
                gid: 1000
            }
        );

        let qid = qid(0x02, 0, 44);
        let symlink_response = p9_rsymlink(16, qid).encode().unwrap();
        assert_eq!(&symlink_response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rsymlink(&P9Frame::decode(&symlink_response).unwrap()).unwrap(),
            qid
        );

        let readlink = p9_treadlink(17, 2).encode().unwrap();
        assert_eq!(&readlink[..4], &11_u32.to_le_bytes());
        assert_eq!(
            p9_decode_treadlink(&P9Frame::decode(&readlink).unwrap()).unwrap(),
            P9ReadLink { fid: 2 }
        );

        let readlink_response = p9_rreadlink(17, "target.txt").unwrap().encode().unwrap();
        assert_eq!(&readlink_response[..4], &19_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rreadlink(&P9Frame::decode(&readlink_response).unwrap()).unwrap(),
            "target.txt"
        );
    }

    #[test]
    fn mknod_round_trips_special_file_payload() {
        let frame = p9_tmknod(18, 1, "tty0", 0o020620, 4, 0, 5)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(frame[4], P9_TMKNOD);
        assert_eq!(
            p9_decode_tmknod(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Mknod {
                dir_fid: 1,
                name: "tty0".to_owned(),
                mode: 0o020620,
                major: 4,
                minor: 0,
                gid: 5
            }
        );

        let qid = qid(0, 0, 44);
        let response = p9_rmknod(18, qid).encode().unwrap();
        assert_eq!(&response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rmknod(&P9Frame::decode(&response).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn getattr_round_trips_fixed_9p2000_l_payload() {
        let frame = p9_tgetattr(11, 77, 0x1234_5678_90ab_cdef).encode().unwrap();
        assert_eq!(&frame[..4], &19_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tgetattr(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9GetAttr {
                fid: 77,
                request_mask: 0x1234_5678_90ab_cdef
            }
        );

        let attr = P9Attr {
            valid: 0x200,
            qid: qid(0x80, 1, 2),
            mode: 0o040755,
            uid: 3,
            gid: 4,
            nlink: 5,
            rdev: 6,
            size: 7,
            block_size: 8,
            blocks: 9,
            atime_seconds: 10,
            atime_nanoseconds: 11,
            mtime_seconds: 12,
            mtime_nanoseconds: 13,
            ctime_seconds: 14,
            ctime_nanoseconds: 15,
            btime_seconds: 16,
            btime_nanoseconds: 17,
            generation: 18,
            data_version: 19,
        };
        let response = p9_rgetattr(11, &attr).encode().unwrap();
        assert_eq!(&response[..4], &160_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rgetattr(&P9Frame::decode(&response).unwrap()).unwrap(),
            attr
        );
    }

    #[test]
    fn setattr_round_trips_fixed_9p2000_l_payload() {
        let attr = P9SetAttr {
            permissions: 0o600,
            uid: 1000,
            gid: 1001,
            size: 44,
            atime_seconds: 5,
            atime_nanoseconds: 6,
            mtime_seconds: 7,
            mtime_nanoseconds: 8,
        };
        let valid = P9_SETATTR_PERMISSIONS
            | P9_SETATTR_SIZE
            | P9_SETATTR_ATIME
            | P9_SETATTR_ATIME_NOT_SYSTEM_TIME
            | P9_SETATTR_MTIME
            | P9_SETATTR_MTIME_NOT_SYSTEM_TIME;
        let frame = p9_tsetattr(12, 77, valid, &attr).encode().unwrap();
        assert_eq!(&frame[..4], &67_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tsetattr(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9SetAttrRequest {
                fid: 77,
                valid,
                attr
            }
        );

        let response = p9_rsetattr(12).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        p9_decode_rsetattr(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn xattrwalk_and_xattrcreate_round_trip() {
        let walk = p9_txattrwalk(30, 10, 11, "user.foo")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(walk[4], P9_TXATTRWALK);
        assert_eq!(
            p9_decode_txattrwalk(&P9Frame::decode(&walk).unwrap()).unwrap(),
            P9XattrWalk {
                fid: 10,
                newfid: 11,
                name: "user.foo".to_owned()
            }
        );

        let walk_response = p9_rxattrwalk(30, 12).encode().unwrap();
        assert_eq!(
            p9_decode_rxattrwalk(&P9Frame::decode(&walk_response).unwrap()).unwrap(),
            12
        );

        let create = p9_txattrcreate(31, 10, "user.foo", 12, 1)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(create[4], P9_TXATTRCREATE);
        assert_eq!(
            p9_decode_txattrcreate(&P9Frame::decode(&create).unwrap()).unwrap(),
            P9XattrCreate {
                fid: 10,
                name: "user.foo".to_owned(),
                attr_size: 12,
                flags: 1
            }
        );

        let create_response = p9_rxattrcreate(31).encode().unwrap();
        p9_decode_rxattrcreate(&P9Frame::decode(&create_response).unwrap()).unwrap();
    }

    #[test]
    fn readdir_round_trips_offsets_types_and_counted_stream() {
        let frame = p9_treaddir(10, 66, 2, 4096).encode().unwrap();
        assert_eq!(
            p9_decode_treaddir(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9ReadDir {
                fid: 66,
                offset: 2,
                count: 4096
            }
        );

        let entries = vec![
            P9DirEntry {
                qid: qid(0x80, 1, 2),
                offset: 1,
                dirent_type: 4,
                name: "bin".to_owned(),
            },
            P9DirEntry {
                qid: qid(0, 3, 4),
                offset: 2,
                dirent_type: 8,
                name: "hello.txt".to_owned(),
            },
        ];
        assert_eq!(p9_dir_entry_encoded_len(&entries[0]).unwrap(), 27);
        assert_eq!(p9_dir_entry_encoded_len(&entries[1]).unwrap(), 33);

        let response = p9_rreaddir(10, &entries).unwrap();
        let encoded = response.encode().unwrap();
        assert_eq!(&encoded[..4], &71_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rreaddir(&P9Frame::decode(&encoded).unwrap()).unwrap(),
            entries
        );
    }

    #[test]
    fn mkdir_renameat_and_unlinkat_round_trip() {
        let mkdir = p9_tmkdir(13, 1, "new-dir", 0o040755, 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&mkdir[..4], &28_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tmkdir(&P9Frame::decode(&mkdir).unwrap()).unwrap(),
            P9Mkdir {
                dir_fid: 1,
                name: "new-dir".to_owned(),
                mode: 0o040755,
                gid: 1000
            }
        );
        let dir_qid = qid(0x80, 0, 44);
        let mkdir_response = p9_rmkdir(13, dir_qid).encode().unwrap();
        assert_eq!(&mkdir_response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rmkdir(&P9Frame::decode(&mkdir_response).unwrap()).unwrap(),
            dir_qid
        );

        let rename = p9_trenameat(14, 1, "old.txt", 2, "new.txt")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&rename[..4], &33_u32.to_le_bytes());
        assert_eq!(
            p9_decode_trenameat(&P9Frame::decode(&rename).unwrap()).unwrap(),
            P9RenameAt {
                old_dir_fid: 1,
                old_name: "old.txt".to_owned(),
                new_dir_fid: 2,
                new_name: "new.txt".to_owned()
            }
        );
        let rename_response = p9_rrenameat(14).encode().unwrap();
        assert_eq!(&rename_response[..4], &7_u32.to_le_bytes());
        p9_decode_rrenameat(&P9Frame::decode(&rename_response).unwrap()).unwrap();

        let unlink = p9_tunlinkat(15, 2, "gone", 0x200)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&unlink[..4], &21_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tunlinkat(&P9Frame::decode(&unlink).unwrap()).unwrap(),
            P9UnlinkAt {
                dir_fid: 2,
                name: "gone".to_owned(),
                flags: 0x200
            }
        );
        let unlink_response = p9_runlinkat(15).encode().unwrap();
        assert_eq!(&unlink_response[..4], &7_u32.to_le_bytes());
        p9_decode_runlinkat(&P9Frame::decode(&unlink_response).unwrap()).unwrap();
    }

    #[test]
    fn legacy_rename_round_trips_fid_dir_fid_and_name() {
        let rename = p9_trename(20, 2, 1, "renamed.txt")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&rename[..4], &28_u32.to_le_bytes());
        assert_eq!(rename[4], P9_TRENAME);
        assert_eq!(
            p9_decode_trename(&P9Frame::decode(&rename).unwrap()).unwrap(),
            P9Rename {
                fid: 2,
                dir_fid: 1,
                name: "renamed.txt".to_owned()
            }
        );

        let response = p9_rrename(20).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RRENAME);
        p9_decode_rrename(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn legacy_remove_round_trips_fid_and_empty_response() {
        let remove = p9_tremove(122, 3).encode().unwrap();
        assert_eq!(&remove[..4], &11_u32.to_le_bytes());
        assert_eq!(remove[4], P9_TREMOVE);
        assert_eq!(
            p9_decode_tremove(&P9Frame::decode(&remove).unwrap()).unwrap(),
            P9Remove { fid: 3 }
        );

        let response = p9_rremove(122).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RREMOVE);
        p9_decode_rremove(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn link_round_trips_target_fid_and_name() {
        let link = p9_tlink(70, 1, 2, "hard.txt").unwrap().encode().unwrap();
        assert_eq!(link[4], P9_TLINK);
        assert_eq!(
            p9_decode_tlink(&P9Frame::decode(&link).unwrap()).unwrap(),
            P9Link {
                dir_fid: 1,
                fid: 2,
                name: "hard.txt".to_owned()
            }
        );

        let response = p9_rlink(70).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        p9_decode_rlink(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn read_and_write_round_trip_offsets_counts_and_data() {
        let read = p9_tread(7, 33, 0x0102_0304_0506_0708, 4096)
            .encode()
            .unwrap();
        assert_eq!(
            p9_decode_tread(&P9Frame::decode(&read).unwrap()).unwrap(),
            P9Read {
                fid: 33,
                offset: 0x0102_0304_0506_0708,
                count: 4096
            }
        );

        let data = b"hello 9p";
        let read_response = p9_rread(7, data).unwrap().encode().unwrap();
        assert_eq!(
            p9_decode_rread(&P9Frame::decode(&read_response).unwrap()).unwrap(),
            data
        );

        let write = p9_twrite(8, 44, 99, data).unwrap().encode().unwrap();
        assert_eq!(
            p9_decode_twrite(&P9Frame::decode(&write).unwrap()).unwrap(),
            P9Write {
                fid: 44,
                offset: 99,
                data: data.to_vec()
            }
        );

        let write_response = p9_rwrite(8, data.len() as u32).encode().unwrap();
        assert_eq!(
            p9_decode_rwrite(&P9Frame::decode(&write_response).unwrap()).unwrap(),
            data.len() as u32
        );
    }

    #[test]
    fn clunk_round_trips_empty_response() {
        let frame = p9_tclunk(9, 55).encode().unwrap();
        assert_eq!(
            p9_decode_tclunk(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Clunk { fid: 55 }
        );

        let response = p9_rclunk(9).encode().unwrap();
        assert_eq!(
            p9_decode_rclunk(&P9Frame::decode(&response).unwrap()).unwrap(),
            ()
        );
    }

    #[test]
    fn typed_decoders_reject_trailing_payloads() {
        let mut frame = p9_tlopen(1, 2, 3);
        frame.payload.push(99);

        assert_eq!(
            p9_decode_tlopen(&frame).unwrap_err(),
            P9Error::TrailingPayload { count: 1 }
        );
    }

    #[test]
    fn counted_data_decoders_reject_short_payloads() {
        let frame = P9Frame::new(P9_RREAD, 1, vec![4, 0, 0, 0, b'a']);

        assert_eq!(
            p9_decode_rread(&frame).unwrap_err(),
            P9Error::UnexpectedEof {
                needed: 4,
                remaining: 1
            }
        );
    }

    fn qid(qid_type: u8, version: u32, path: u64) -> P9Qid {
        P9Qid {
            qid_type,
            version,
            path,
        }
    }
}
