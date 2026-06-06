use std::fmt;

use super::P9_HEADER_LEN;

const P9_SIZE_FIELD_LEN: usize = 4;

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
    /// A walk name vector cannot fit in the 9P u16 name count field.
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
    pub(super) message_type: u8,
    pub(super) tag: u16,
    pub(super) payload: Vec<u8>,
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
        let Some(declared) = p9_declared_size(bytes)? else {
            return Err(P9Error::ShortFrame { len: bytes.len() });
        };
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
    if bytes.len() < P9_SIZE_FIELD_LEN {
        return Ok(None);
    }
    let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p9_error_display_messages_are_stable() {
        let cases = [
            (
                P9Error::ShortFrame { len: 3 },
                "9P frame is too short: 3 bytes",
            ),
            (
                P9Error::InvalidFrameSize { size: 6 },
                "9P frame declares invalid size 6",
            ),
            (
                P9Error::FrameSizeMismatch {
                    declared: 9,
                    actual: 8,
                },
                "9P frame declares 9 bytes but input has 8 bytes",
            ),
            (
                P9Error::FrameTooLarge { size: usize::MAX },
                "9P frame is too large for u32 size field: 18446744073709551615 bytes",
            ),
            (
                P9Error::UnexpectedEof {
                    needed: 4,
                    remaining: 2,
                },
                "9P payload needs 4 bytes but only 2 remain",
            ),
            (
                P9Error::TrailingPayload { count: 1 },
                "9P payload has 1 trailing bytes",
            ),
            (P9Error::InvalidUtf8, "9P string is not valid UTF-8"),
            (
                P9Error::StringTooLong { len: 65_536 },
                "9P string is too long for u16 length: 65536 bytes",
            ),
            (
                P9Error::DataTooLong { len: usize::MAX },
                "9P data is too long for u32 count: 18446744073709551615 bytes",
            ),
            (
                P9Error::TooManyWalkNames { count: 65_536 },
                "9P walk has too many names for u16 count: 65536",
            ),
            (
                P9Error::UnexpectedMessageType {
                    expected: 100,
                    actual: 101,
                },
                "9P frame has message type 101, expected 100",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn p9_frame_round_trips_header_and_payload() {
        let frame = P9Frame::new(100, 7, vec![1, 2, 3, 4]);
        assert_eq!(frame.message_type(), 100);
        assert_eq!(frame.tag(), 7);
        assert_eq!(frame.payload(), &[1, 2, 3, 4]);

        let encoded = frame.encode().unwrap();
        assert_eq!(p9_declared_size(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(p9_declared_size(&encoded[..3]).unwrap(), None);
        assert_eq!(p9_tag_from_frame_bytes(&encoded).unwrap(), 7);
        assert_eq!(P9Frame::decode(&encoded).unwrap(), frame);
    }

    #[test]
    fn p9_frame_buffer_keeps_partial_frame_bytes() {
        let frame = P9Frame::new(100, 1, vec![9, 8]).encode().unwrap();
        let mut buffer = P9FrameBuffer::new();

        assert!(buffer.push(&frame[..4]).unwrap().is_empty());
        assert_eq!(buffer.buffered_len(), 4);
        let frames = buffer.push(&frame[4..]).unwrap();

        assert_eq!(frames, vec![P9Frame::decode(&frame).unwrap()]);
        assert_eq!(buffer.buffered_len(), 0);
    }
}
