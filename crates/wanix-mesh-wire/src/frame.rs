//! Length-prefixed `postcard` framing over a sync byte stream.
//!
//! Every frame is a `4-byte little-endian length prefix` followed by a
//! `postcard` body, exactly the ceiling discipline already shipping in
//! `crates/wanix-9p-client/src/transport.rs` (`read_one_frame`) and
//! `crates/wanix-cpu/src/wire.rs`. The reader reads the 4-byte prefix first and
//! rejects any body over an explicit ceiling **before allocating**, then reads
//! exactly that many bytes and `postcard`-decodes. This protects an
//! importer/exporter from a hostile peer forcing an unbounded allocation.
//!
//! There are no tags and no `msize`: on the native mesh wire the QUIC stream is
//! the transaction, and QUIC flow control bounds memory in flight. These
//! ceilings only bound a single decoded frame.

use std::io::{Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Upper bound on a single control frame body (requests, responses, metadata,
/// one `readdir` page). Frames over this are rejected before allocation.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

/// Upper bound on a single `Chunk`/`Write` payload. This is the server-side
/// read-chunk cap, independent of a client's requested `Read{max}`: a client
/// looping `Read` with a huge `max` must not force a large server buffer, so the
/// server clamps each `Chunk` to `min(client_max, MAX_CHUNK_LEN, iounit_hint)`.
/// The `iounit_hint` carried in the open response is advisory only.
pub const MAX_CHUNK_LEN: usize = 256 * 1024;

/// Number of bytes in the little-endian length prefix.
const LEN_PREFIX_LEN: usize = 4;

/// A framing error: a transport failure, a truncated frame, an over-ceiling
/// length prefix, or a `postcard` encode/decode failure.
#[derive(Debug)]
pub enum FrameError {
    /// The transport read or write failed, or the stream closed mid-frame.
    Io(std::io::Error),
    /// The declared frame length exceeded the supplied ceiling, or the encoded
    /// body did not fit in the 4-byte prefix.
    TooLong {
        /// The declared (or required) body length in bytes.
        declared: usize,
        /// The ceiling that was exceeded.
        limit: usize,
    },
    /// The frame body could not be `postcard`-encoded or decoded.
    Codec(postcard::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "mesh wire transport error: {error}"),
            Self::TooLong { declared, limit } => write!(
                f,
                "mesh wire frame of {declared} bytes exceeds the {limit}-byte ceiling"
            ),
            Self::Codec(error) => write!(f, "mesh wire codec error: {error}"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::TooLong { .. } => None,
        }
    }
}

impl From<std::io::Error> for FrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<postcard::Error> for FrameError {
    fn from(error: postcard::Error) -> Self {
        Self::Codec(error)
    }
}

/// Result of a framing operation.
pub type FrameResult<T> = Result<T, FrameError>;

/// Encodes `value` with `postcard` and writes it as one length-prefixed frame,
/// then flushes.
///
/// `max_len` bounds the encoded body so a caller cannot accidentally emit a
/// frame the peer would reject; the body is encoded first and its length checked
/// against both `max_len` and `u32::MAX` before any prefix is written.
///
/// # Errors
///
/// Returns [`FrameError::Codec`] when the body cannot be encoded,
/// [`FrameError::TooLong`] when the encoded body exceeds `max_len` (or does not
/// fit a `u32` prefix), and [`FrameError::Io`] when the write or flush fails.
pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    value: &T,
    max_len: usize,
) -> FrameResult<()> {
    let body = postcard::to_allocvec(value)?;
    if body.len() > max_len {
        return Err(FrameError::TooLong {
            declared: body.len(),
            limit: max_len,
        });
    }
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLong {
        declared: body.len(),
        limit: max_len,
    })?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Reads exactly one length-prefixed frame and `postcard`-decodes it as `T`.
///
/// The 4-byte little-endian prefix is read first; a declared length over
/// `max_len` is rejected with [`FrameError::TooLong`] **before** the body buffer
/// is allocated, so a peer cannot force an unbounded allocation by declaring a
/// huge frame. Otherwise exactly `len` bytes are read and decoded.
///
/// # Errors
///
/// Returns [`FrameError::Io`] on a transport failure or a stream that closes
/// before a full frame arrives, [`FrameError::TooLong`] when the declared length
/// exceeds `max_len`, and [`FrameError::Codec`] when the body fails to decode.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R, max_len: usize) -> FrameResult<T> {
    let mut prefix = [0_u8; LEN_PREFIX_LEN];
    read_exact(reader, &mut prefix)?;
    let declared = u32::from_le_bytes(prefix) as usize;
    if declared > max_len {
        // Reject before allocating the body buffer.
        return Err(FrameError::TooLong {
            declared,
            limit: max_len,
        });
    }
    let mut body = vec![0_u8; declared];
    read_exact(reader, &mut body)?;
    Ok(postcard::from_bytes(&body)?)
}

/// Fills `buf` fully, mapping a clean mid-frame EOF to an explicit I/O error.
fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> FrameResult<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let count = reader.read(&mut buf[filled..])?;
        if count == 0 {
            return Err(FrameError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "mesh wire transport closed before a full frame arrived",
            )));
        }
        filled += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{FrameError, MAX_FRAME_LEN, read_frame, write_frame};
    use crate::error::WireFsError;
    use crate::value::WireMetadata;
    use wanix_fs::{FileType, Metadata, MetadataTimes};

    #[test]
    fn frame_round_trips_a_value() {
        let metadata =
            Metadata::new_with_links(FileType::File, 128, 0o644, 1, MetadataTimes::new(1, 2, 3));
        let wire = WireMetadata::from(&metadata);
        let mut buf = Vec::new();
        write_frame(&mut buf, &wire, MAX_FRAME_LEN).unwrap();
        let mut cursor = Cursor::new(buf);
        let decoded: WireMetadata = read_frame(&mut cursor, MAX_FRAME_LEN).unwrap();
        assert_eq!(decoded, wire);
    }

    #[test]
    fn frame_round_trips_a_typed_error() {
        let wire = WireFsError::InvalidPath("a/../b".to_owned());
        let mut buf = Vec::new();
        write_frame(&mut buf, &wire, MAX_FRAME_LEN).unwrap();
        let mut cursor = Cursor::new(buf);
        let decoded: WireFsError = read_frame(&mut cursor, MAX_FRAME_LEN).unwrap();
        assert_eq!(decoded, wire);
    }

    #[test]
    fn an_over_ceiling_prefix_is_rejected_without_allocating() {
        // Declare a 1 GiB frame but supply only the 4-byte prefix; the ceiling
        // must fire before the body is read, so the short stream never matters
        // and no gigabyte buffer is ever allocated.
        let huge = (1_073_741_824_u32).to_le_bytes().to_vec();
        let mut cursor = Cursor::new(huge);
        let error = read_frame::<_, WireFsError>(&mut cursor, MAX_FRAME_LEN).unwrap_err();
        match error {
            FrameError::TooLong { declared, limit } => {
                assert_eq!(declared, 1_073_741_824);
                assert_eq!(limit, MAX_FRAME_LEN);
            }
            other => panic!("expected TooLong, got {other:?}"),
        }
    }

    #[test]
    fn a_frame_exactly_at_the_ceiling_passes_the_prefix_check() {
        // A prefix equal to the ceiling is allowed (the bound is strict-greater);
        // here the body is truncated, so the body read fails with Io, proving the
        // prefix check itself did not reject a ceiling-sized declared length.
        let at_limit = u32::try_from(MAX_FRAME_LEN).unwrap().to_le_bytes().to_vec();
        let mut cursor = Cursor::new(at_limit);
        let error = read_frame::<_, WireFsError>(&mut cursor, MAX_FRAME_LEN).unwrap_err();
        assert!(matches!(error, FrameError::Io(_)));
    }

    #[test]
    fn writing_a_body_over_the_ceiling_is_rejected() {
        // A WireFsError::Other with a body larger than the ceiling cannot be
        // written; the encode-then-check guard fires before any prefix is sent.
        let wire = WireFsError::Other("x".repeat(64));
        let mut buf = Vec::new();
        let error = write_frame(&mut buf, &wire, 8).unwrap_err();
        assert!(matches!(error, FrameError::TooLong { .. }));
        assert!(
            buf.is_empty(),
            "no bytes should be written for a too-long frame"
        );
    }

    #[test]
    fn a_truncated_prefix_maps_to_io() {
        let mut cursor = Cursor::new(vec![0_u8, 0_u8]); // only 2 of 4 prefix bytes
        let error = read_frame::<_, WireFsError>(&mut cursor, MAX_FRAME_LEN).unwrap_err();
        assert!(matches!(error, FrameError::Io(_)));
    }
}
