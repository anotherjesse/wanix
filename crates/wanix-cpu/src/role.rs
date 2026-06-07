//! The 1-byte stream-role discriminator that orders a job's two bidi streams.
//!
//! A CPU job uses two bidirectional streams: a **control** stream (out-of-band
//! [`crate::CpuEvent`] lifecycle) and an **export** stream (the caller's reverse
//! 9P namespace). Over QUIC the two streams do not arrive at the acceptor in
//! their open order — an `open_bi` stream is invisible to the peer's `accept_bi`
//! until its opener writes a first byte, so *first-write order*, not open order,
//! decides which stream the acceptor sees first.
//!
//! To make the pairing unambiguous, the caller writes one role byte on each
//! stream immediately after opening it, and the acceptor reads the first byte of
//! each accepted stream to classify it. This is the cpu correction baked into v1:
//! never assume "the first accepted stream is control".

use std::io::{Read, Write};

/// Role byte written first on the control stream.
pub const ROLE_CONTROL: u8 = 0;
/// Role byte written first on the export stream.
pub const ROLE_EXPORT: u8 = 1;

/// Which stream of a CPU job a freshly accepted bidi stream is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRole {
    /// The control stream, carrying [`crate::CpuEvent`] job lifecycle frames.
    Control,
    /// The export stream, carrying the caller's reverse 9P namespace.
    Export,
}

impl StreamRole {
    /// Returns the wire role byte for this role.
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            Self::Control => ROLE_CONTROL,
            Self::Export => ROLE_EXPORT,
        }
    }

    /// Classifies a wire role byte, rejecting any value that is not a known role.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when `byte` is neither [`ROLE_CONTROL`] nor
    /// [`ROLE_EXPORT`].
    pub fn from_byte(byte: u8) -> std::io::Result<Self> {
        match byte {
            ROLE_CONTROL => Ok(Self::Control),
            ROLE_EXPORT => Ok(Self::Export),
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown CPU stream role byte {other}"),
            )),
        }
    }
}

/// Writes the role byte for `role` as the first byte on `writer`.
///
/// The caller writes this immediately after `open_bi` so the acceptor's
/// `accept_bi` resolves (the first byte is what makes the stream visible to the
/// peer) and so the role is unambiguous regardless of stream arrival order.
///
/// # Errors
///
/// Returns an I/O error when the write or flush fails.
pub fn write_role<W: Write>(writer: &mut W, role: StreamRole) -> std::io::Result<()> {
    writer.write_all(&[role.as_byte()])?;
    writer.flush()
}

/// Reads and classifies the leading role byte from `reader`.
///
/// The acceptor calls this on each accepted stream before doing anything else
/// with it, then routes the stream to the control or export handler accordingly.
///
/// # Errors
///
/// Returns an I/O error when the stream ends before the role byte arrives or the
/// byte is not a known role.
pub fn read_role<R: Read>(reader: &mut R) -> std::io::Result<StreamRole> {
    let mut byte = [0_u8; 1];
    reader.read_exact(&mut byte).map_err(|err| {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "CPU stream closed before its role byte arrived",
            )
        } else {
            err
        }
    })?;
    StreamRole::from_byte(byte[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_role_round_trips() {
        let mut buf = Vec::new();
        write_role(&mut buf, StreamRole::Control).unwrap();
        assert_eq!(buf, vec![ROLE_CONTROL]);
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_role(&mut cursor).unwrap(), StreamRole::Control);
    }

    #[test]
    fn export_role_round_trips() {
        let mut buf = Vec::new();
        write_role(&mut buf, StreamRole::Export).unwrap();
        assert_eq!(buf, vec![ROLE_EXPORT]);
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_role(&mut cursor).unwrap(), StreamRole::Export);
    }

    #[test]
    fn an_unknown_role_byte_is_rejected() {
        let mut cursor = std::io::Cursor::new(vec![42_u8]);
        assert!(read_role(&mut cursor).is_err());
    }

    #[test]
    fn an_empty_stream_is_an_eof_error() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let err = read_role(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
