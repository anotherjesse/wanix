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

/// 9P2000.L `Tlopen` message type.
pub const P9_TLOPEN: u8 = 12;

/// 9P2000.L `Rlopen` message type.
pub const P9_RLOPEN: u8 = 13;

/// 9P2000.L `Treaddir` message type.
pub const P9_TREADDIR: u8 = 40;

/// 9P2000.L `Rreaddir` message type.
pub const P9_RREADDIR: u8 = 41;

/// 9P `Tversion` message type.
pub const P9_TVERSION: u8 = 100;

/// 9P `Rversion` message type.
pub const P9_RVERSION: u8 = 101;

/// 9P `Tattach` message type.
pub const P9_TATTACH: u8 = 104;

/// 9P `Rattach` message type.
pub const P9_RATTACH: u8 = 105;

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

/// Returns a stable name for message types the Rust port currently identifies.
#[must_use]
pub const fn p9_message_type_name(message_type: u8) -> Option<&'static str> {
    match message_type {
        P9_RLERROR => Some("Rlerror"),
        P9_TLOPEN => Some("Tlopen"),
        P9_RLOPEN => Some("Rlopen"),
        P9_TREADDIR => Some("Treaddir"),
        P9_RREADDIR => Some("Rreaddir"),
        P9_TVERSION => Some("Tversion"),
        P9_RVERSION => Some("Rversion"),
        P9_TATTACH => Some("Tattach"),
        P9_RATTACH => Some("Rattach"),
        P9_TWALK => Some("Twalk"),
        P9_RWALK => Some("Rwalk"),
        P9_TREAD => Some("Tread"),
        P9_RREAD => Some("Rread"),
        P9_TWRITE => Some("Twrite"),
        P9_RWRITE => Some("Rwrite"),
        P9_TCLUNK => Some("Tclunk"),
        P9_RCLUNK => Some("Rclunk"),
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

/// Builds an `Rlerror` frame.
#[must_use]
pub fn p9_rlerror(tag: u16, ecode: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, ecode);
    P9Frame::new(P9_RLERROR, tag, payload)
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
        assert_eq!(p9_message_type_name(P9_TLOPEN), Some("Tlopen"));
        assert_eq!(p9_message_type_name(P9_RLOPEN), Some("Rlopen"));
        assert_eq!(p9_message_type_name(P9_TREADDIR), Some("Treaddir"));
        assert_eq!(p9_message_type_name(P9_RREADDIR), Some("Rreaddir"));
        assert_eq!(p9_message_type_name(P9_TATTACH), Some("Tattach"));
        assert_eq!(p9_message_type_name(P9_RATTACH), Some("Rattach"));
        assert_eq!(p9_message_type_name(P9_TWALK), Some("Twalk"));
        assert_eq!(p9_message_type_name(P9_RWALK), Some("Rwalk"));
        assert_eq!(p9_message_type_name(P9_TREAD), Some("Tread"));
        assert_eq!(p9_message_type_name(P9_RREAD), Some("Rread"));
        assert_eq!(p9_message_type_name(P9_TWRITE), Some("Twrite"));
        assert_eq!(p9_message_type_name(P9_RWRITE), Some("Rwrite"));
        assert_eq!(p9_message_type_name(P9_TCLUNK), Some("Tclunk"));
        assert_eq!(p9_message_type_name(P9_RCLUNK), Some("Rclunk"));
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
