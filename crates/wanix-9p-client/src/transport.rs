//! Byte-stream transport abstraction and framed reads for the 9P client.
//!
//! The client is transport-agnostic: it speaks 9P over any blocking,
//! bidirectional byte stream. [`Duplex`] is the single requirement, satisfied by
//! a TCP stream, a Unix socket, a `std::io::pipe` pair joined into one handle, or
//! any user-supplied framed channel. This crate brings no transport of its own.
//!
//! The client does its own framing rather than buffering through
//! `wanix_protocol::P9FrameBuffer`: it reads the 4-byte size prefix first,
//! rejects any frame larger than the negotiated `msize` before allocating its
//! body, then reads exactly that many bytes. This is the client-side frame-size
//! ceiling that protects an importer from a hostile or buggy server declaring a
//! multi-gigabyte reply.

use std::io::{Read, Write};

use wanix_protocol::{P9_HEADER_LEN, P9Frame};

use crate::error::{ClientError, ClientResult};

/// Bytes of the 9P size prefix that precede the rest of every frame.
const SIZE_PREFIX_LEN: usize = 4;

/// A blocking, bidirectional byte stream the 9P client can run a session over.
///
/// Any type that is both [`Read`] and [`Write`] and [`Send`] is a `Duplex`
/// through the blanket implementation below. The client never assumes framing,
/// readiness, or async behavior beyond the synchronous [`Read`]/[`Write`]
/// contract.
pub trait Duplex: Read + Write + Send {}

impl<T: Read + Write + Send> Duplex for T {}

/// Reads exactly one complete 9P frame, enforcing the negotiated size ceiling.
///
/// The 4-byte size prefix is read first. A declared size below the 9P header or
/// above `max_msize` is rejected with [`ClientError::Poisoned`] before the body
/// is allocated, so a server cannot force an unbounded allocation. Otherwise the
/// remaining `size - 4` bytes are read and decoded into a [`P9Frame`].
///
/// # Errors
///
/// Returns [`ClientError::Io`] on a transport failure or premature EOF,
/// [`ClientError::Protocol`] when the assembled bytes fail to decode, and
/// [`ClientError::Poisoned`] when the declared size is impossible or exceeds the
/// negotiated `max_msize`.
pub fn read_one_frame<R: Read>(reader: &mut R, max_msize: u32) -> ClientResult<P9Frame> {
    let mut prefix = [0_u8; SIZE_PREFIX_LEN];
    read_exact(reader, &mut prefix)?;
    let size = u32::from_le_bytes(prefix);
    if (size as usize) < P9_HEADER_LEN {
        return Err(ClientError::Poisoned(format!(
            "server declared a {size}-byte frame smaller than the 9P header"
        )));
    }
    if size > max_msize {
        return Err(ClientError::Poisoned(format!(
            "server declared a {size}-byte frame exceeding the negotiated msize {max_msize}"
        )));
    }
    let mut frame_bytes = vec![0_u8; size as usize];
    frame_bytes[..SIZE_PREFIX_LEN].copy_from_slice(&prefix);
    read_exact(reader, &mut frame_bytes[SIZE_PREFIX_LEN..])?;
    Ok(P9Frame::decode(&frame_bytes)?)
}

/// Fills `buf` fully, mapping a clean EOF to an explicit transport error.
fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> ClientResult<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let count = reader.read(&mut buf[filled..])?;
        if count == 0 {
            return Err(ClientError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "9P transport closed before a full reply frame arrived",
            )));
        }
        filled += count;
    }
    Ok(())
}

/// Encodes `frame` and writes every byte to `writer`, then flushes.
///
/// # Errors
///
/// Returns [`ClientError::Protocol`] when the frame cannot be encoded and
/// [`ClientError::Io`] when the transport write or flush fails.
pub fn write_frame<W: Write>(writer: &mut W, frame: &P9Frame) -> ClientResult<()> {
    let bytes = frame.encode()?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use wanix_protocol::{P9_VERSION_9P2000_L, p9_rversion};

    use super::*;

    #[test]
    fn reads_a_well_formed_frame() {
        let frame = p9_rversion(7, 8192, P9_VERSION_9P2000_L).unwrap();
        let bytes = frame.encode().unwrap();
        let mut cursor = Cursor::new(bytes);
        let decoded = read_one_frame(&mut cursor, 8192).unwrap();
        assert_eq!(decoded.tag(), 7);
    }

    #[test]
    fn rejects_a_frame_larger_than_msize_without_reading_body() {
        // Declare a 1 GiB frame but supply only the prefix; the ceiling must
        // fire before the body is read, so the short stream never matters.
        let mut bytes = (1_073_741_824_u32).to_le_bytes().to_vec();
        bytes.push(101); // a plausible message type byte
        let mut cursor = Cursor::new(bytes);
        let error = read_one_frame(&mut cursor, 8192).unwrap_err();
        assert!(matches!(error, ClientError::Poisoned(_)));
    }

    #[test]
    fn rejects_a_frame_smaller_than_the_header() {
        let bytes = (5_u32).to_le_bytes().to_vec();
        let mut cursor = Cursor::new(bytes);
        let error = read_one_frame(&mut cursor, 8192).unwrap_err();
        assert!(matches!(error, ClientError::Poisoned(_)));
    }

    #[test]
    fn maps_premature_eof_to_io_error() {
        let frame = p9_rversion(1, 8192, P9_VERSION_9P2000_L).unwrap();
        let mut bytes = frame.encode().unwrap();
        bytes.truncate(bytes.len() - 2);
        let mut cursor = Cursor::new(bytes);
        let error = read_one_frame(&mut cursor, 8192).unwrap_err();
        assert!(matches!(error, ClientError::Io(_)));
    }
}
