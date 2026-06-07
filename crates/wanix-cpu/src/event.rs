//! The CPU control-plane wire format: typed, length-prefixed [`CpuEvent`] frames.
//!
//! Once a job's role-sorted control stream is established, the acceptor (node Y)
//! delivers the job's result over it as a sequence of `CpuEvent` frames, and the
//! caller drives the control stream by reading them. The framing is deliberately
//! tiny and self-contained — it is *not* 9P — because the control stream carries
//! only out-of-band job lifecycle events, while the export stream carries the
//! full 9P namespace traffic.
//!
//! # Wire layout
//!
//! Each frame is `kind(1) || len(4, little-endian) || payload(len)`. The 4-byte
//! length is bounded by [`MAX_EVENT_PAYLOAD`] on decode so a hostile or buggy
//! peer cannot force an unbounded allocation — the same frame-ceiling discipline
//! the 9P client applies to imported frames.
//!
//! # Batch-after-`start` honesty
//!
//! The current task model runs a guest to completion inside
//! [`wanix_task::TaskDriver::start`] and only then has its buffered stdout. So v1
//! delivers [`CpuEvent::Stdout`]/[`CpuEvent::Stderr`]/[`CpuEvent::Exit`] as a
//! single batch *after* `start` returns, not incrementally. Incremental
//! streaming (a streaming-stdout `File` that pushes frames during eval) is a
//! named follow-up, not implied by this format.

use std::io::{Read, Write};

/// Upper bound on a single [`CpuEvent`] payload, enforced on decode.
///
/// A stdout/stderr batch larger than this is split across multiple frames by the
/// acceptor. The ceiling protects the reader from an unbounded allocation
/// declared by an untrusted peer.
pub const MAX_EVENT_PAYLOAD: usize = 1 << 20;

/// Frame kind byte for a captured-stdout chunk.
const KIND_STDOUT: u8 = 1;
/// Frame kind byte for a captured-stderr chunk.
const KIND_STDERR: u8 = 2;
/// Frame kind byte for the terminal exit-status event.
const KIND_EXIT: u8 = 3;
/// Frame kind byte for a caller-initiated cancel.
const KIND_CANCEL: u8 = 4;

/// One control-plane event in a CPU job's lifecycle.
///
/// The acceptor emits [`Self::Stdout`], [`Self::Stderr`], and a terminal
/// [`Self::Exit`]; the caller may emit [`Self::Cancel`] to stop *draining* the
/// control stream. Cancel does **not** abort the remote computation — the task
/// driver has no abort hook, so a cancel only tells the caller side to stop
/// reading; the guest on node Y still runs to completion. This limitation is
/// intentional and documented rather than faked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuEvent {
    /// A chunk of the job's captured standard output.
    Stdout(Vec<u8>),
    /// A chunk of the job's captured standard error.
    Stderr(Vec<u8>),
    /// The job's terminal exit status (the parsed `#task` exit code).
    Exit(i32),
    /// A caller-initiated request to stop draining the control stream.
    ///
    /// This is best-effort and stops only the caller's drain loop; it does not
    /// cancel the running guest on the acceptor.
    Cancel,
}

impl CpuEvent {
    /// Writes this event as one length-prefixed frame to `writer`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the underlying write fails.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let (kind, payload) = self.encode_parts();
        writer.write_all(&[kind])?;
        let len = u32::try_from(payload.len()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "CpuEvent payload exceeds u32 length",
            )
        })?;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(&payload)?;
        writer.flush()
    }

    /// Reads one length-prefixed frame from `reader`, or `None` at clean EOF.
    ///
    /// A clean EOF *before* any byte of a frame returns `Ok(None)`, the normal
    /// end of a control stream. An EOF mid-frame is an error.
    ///
    /// # Errors
    ///
    /// Returns an I/O error on a transport failure, a truncated frame, an
    /// unknown kind byte, or a declared length exceeding [`MAX_EVENT_PAYLOAD`].
    pub fn read_from<R: Read>(reader: &mut R) -> std::io::Result<Option<Self>> {
        let mut kind = [0_u8; 1];
        match read_full_or_eof(reader, &mut kind)? {
            ReadOutcome::Eof => return Ok(None),
            ReadOutcome::Filled => {}
        }
        let mut len_bytes = [0_u8; 4];
        read_exact_framed(reader, &mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > MAX_EVENT_PAYLOAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("CpuEvent payload {len} exceeds the {MAX_EVENT_PAYLOAD}-byte ceiling"),
            ));
        }
        let mut payload = vec![0_u8; len];
        read_exact_framed(reader, &mut payload)?;
        Self::decode_parts(kind[0], payload)
    }

    /// Splits this event into its wire kind byte and payload bytes.
    fn encode_parts(&self) -> (u8, Vec<u8>) {
        match self {
            Self::Stdout(bytes) => (KIND_STDOUT, bytes.clone()),
            Self::Stderr(bytes) => (KIND_STDERR, bytes.clone()),
            Self::Exit(code) => (KIND_EXIT, code.to_le_bytes().to_vec()),
            Self::Cancel => (KIND_CANCEL, Vec::new()),
        }
    }

    /// Reassembles an event from a decoded `kind` byte and `payload`.
    fn decode_parts(kind: u8, payload: Vec<u8>) -> std::io::Result<Option<Self>> {
        let event = match kind {
            KIND_STDOUT => Self::Stdout(payload),
            KIND_STDERR => Self::Stderr(payload),
            KIND_EXIT => Self::Exit(decode_exit(&payload)?),
            KIND_CANCEL => Self::Cancel,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown CpuEvent kind byte {other}"),
                ));
            }
        };
        Ok(Some(event))
    }
}

/// Decodes the little-endian `i32` exit payload, rejecting a wrong length.
fn decode_exit(payload: &[u8]) -> std::io::Result<i32> {
    let bytes: [u8; 4] = payload.try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "CpuEvent::Exit payload must be 4 bytes",
        )
    })?;
    Ok(i32::from_le_bytes(bytes))
}

/// Outcome of an attempted full read that tolerates a clean leading EOF.
enum ReadOutcome {
    /// The buffer was filled completely.
    Filled,
    /// EOF arrived before any byte was read.
    Eof,
}

/// Fills `buf`, returning [`ReadOutcome::Eof`] only when EOF precedes byte one.
fn read_full_or_eof<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<ReadOutcome> {
    let mut filled = 0;
    while filled < buf.len() {
        let count = reader.read(&mut buf[filled..])?;
        if count == 0 {
            if filled == 0 {
                return Ok(ReadOutcome::Eof);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "CpuEvent frame truncated at its first field",
            ));
        }
        filled += count;
    }
    Ok(ReadOutcome::Filled)
}

/// Fills `buf` fully, mapping any EOF to a truncated-frame error.
fn read_exact_framed<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<()> {
    reader.read_exact(buf).map_err(|err| {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "CpuEvent frame truncated before its declared length",
            )
        } else {
            err
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(event: &CpuEvent) -> CpuEvent {
        let mut buf = Vec::new();
        event.write_to(&mut buf).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        CpuEvent::read_from(&mut cursor).unwrap().unwrap()
    }

    #[test]
    fn stdout_frame_round_trips() {
        assert_eq!(
            round_trip(&CpuEvent::Stdout(b"hello".to_vec())),
            CpuEvent::Stdout(b"hello".to_vec())
        );
    }

    #[test]
    fn stderr_frame_round_trips() {
        assert_eq!(
            round_trip(&CpuEvent::Stderr(b"boom".to_vec())),
            CpuEvent::Stderr(b"boom".to_vec())
        );
    }

    #[test]
    fn exit_frame_round_trips_a_negative_code() {
        assert_eq!(round_trip(&CpuEvent::Exit(-7)), CpuEvent::Exit(-7));
    }

    #[test]
    fn cancel_frame_round_trips() {
        assert_eq!(round_trip(&CpuEvent::Cancel), CpuEvent::Cancel);
    }

    #[test]
    fn a_stream_of_events_decodes_in_order() {
        let mut buf = Vec::new();
        CpuEvent::Stdout(b"a".to_vec()).write_to(&mut buf).unwrap();
        CpuEvent::Stderr(b"b".to_vec()).write_to(&mut buf).unwrap();
        CpuEvent::Exit(0).write_to(&mut buf).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(
            CpuEvent::read_from(&mut cursor).unwrap(),
            Some(CpuEvent::Stdout(b"a".to_vec()))
        );
        assert_eq!(
            CpuEvent::read_from(&mut cursor).unwrap(),
            Some(CpuEvent::Stderr(b"b".to_vec()))
        );
        assert_eq!(
            CpuEvent::read_from(&mut cursor).unwrap(),
            Some(CpuEvent::Exit(0))
        );
        assert_eq!(CpuEvent::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn clean_eof_at_a_frame_boundary_is_none() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert_eq!(CpuEvent::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn an_unknown_kind_byte_is_rejected() {
        let mut buf = vec![99_u8];
        buf.extend_from_slice(&0_u32.to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        assert!(CpuEvent::read_from(&mut cursor).is_err());
    }

    #[test]
    fn a_payload_over_the_ceiling_is_rejected_without_allocating() {
        // kind=stdout, declared length one past the ceiling, but no body bytes:
        // the ceiling must fire before the (absent) body is read.
        let mut buf = vec![KIND_STDOUT];
        let len = u32::try_from(MAX_EVENT_PAYLOAD + 1).unwrap();
        buf.extend_from_slice(&len.to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let err = CpuEvent::read_from(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_truncated_length_field_is_an_error_not_eof() {
        // One kind byte then EOF: a frame has begun, so this is truncation.
        let mut cursor = std::io::Cursor::new(vec![KIND_EXIT]);
        let err = CpuEvent::read_from(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
