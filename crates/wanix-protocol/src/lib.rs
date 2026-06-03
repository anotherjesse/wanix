//! Wire protocol helpers for Rust Wanix.
//!
//! The first protocol surface is dependency-free 9P framing. It is intentionally
//! below Wanix filesystem policy: callers can split byte streams into tagged
//! 9P frames and round-trip version negotiation before the Rust port grows a
//! full 9P server backed by Wanix namespaces.

use std::fmt;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix wire protocol helpers";

/// 9P2000.L version string used by the existing v86 integration.
pub const P9_VERSION_9P2000_L: &str = "9P2000.L";

/// The 9P `NOTAG` value used by version negotiation.
pub const P9_NOTAG: u16 = 0xffff;

/// Minimum 9P frame size: `size[4] type[1] tag[2]`.
pub const P9_HEADER_LEN: usize = 7;

/// 9P `Rlerror` message type.
pub const P9_RLERROR: u8 = 7;

/// 9P `Tversion` message type.
pub const P9_TVERSION: u8 = 100;

/// 9P `Rversion` message type.
pub const P9_RVERSION: u8 = 101;

/// Returns a stable name for message types the Rust port currently identifies.
#[must_use]
pub const fn p9_message_type_name(message_type: u8) -> Option<&'static str> {
    match message_type {
        P9_RLERROR => Some("Rlerror"),
        P9_TVERSION => Some("Tversion"),
        P9_RVERSION => Some("Rversion"),
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
    if frame.message_type != expected {
        return Err(P9Error::UnexpectedMessageType {
            expected,
            actual: frame.message_type,
        });
    }
    let mut cursor = PayloadCursor::new(&frame.payload);
    let msize = cursor.read_u32()?;
    let version = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9Version { msize, version })
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

    fn read_string(&mut self) -> Result<String, P9Error> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| P9Error::InvalidUtf8)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
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
}
