//! First-frame deadline helpers for native-wire streams, both directions.
//!
//! **Inbound (server)**: an accepted bidi stream holds a session permit and a
//! blocking-pool thread before any frame arrives ([`crate::wire_handler`]),
//! and the sync server's reads are deliberately untimed (the never-EOF
//! idle-read contract). Left unguarded, a peer that opens streams and never
//! completes a request frame pins permits and threads until its connection
//! dies — enough silent streams wedge the endpoint for every other peer.
//! [`read_first_frame`] reads the complete FIRST frame under a deadline
//! *before* the untimed [`wanix_mesh_wire::serve_one`] loop starts, then
//! replays it through [`ReplayDuplex`]: the bound applies exactly once,
//! pre-request, leaving the idle open-file read untimed.
//!
//! **Outbound (client)**: an open-file stream's FIRST reply (the open
//! response) answers immediately on a healthy provider, so it carries the
//! per-op deadline — a silently dead peer fails the open fast. Every LATER
//! reply may legitimately take arbitrarily long (a never-EOF device read
//! parked for data; a synchronous `ctl run` on a job device running the job
//! to completion before its write reply), so [`OpenFileDuplex`] drops the
//! read deadline once the first reply frame is complete and lets QUIC
//! connection liveness (keepalives + idle timeout) bound a dead peer. This is
//! the client mirror of the server's idle-read-vs-in-flight-write asymmetry.

use std::io::{self, Read, Write};
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use tokio::runtime::Handle;

use crate::duplex::{BlockingWriter, block_on_deadline};

/// Bound on assembling the first frame when the serve config carries no
/// per-op deadline (node-served paths always set one; this covers raw
/// handler use).
const FIRST_FRAME_FALLBACK_DEADLINE: Duration = Duration::from_secs(30);

/// The first-frame bound for a serve config's per-op `deadline`.
pub(crate) fn first_frame_deadline(deadline: Option<Duration>) -> Duration {
    deadline.unwrap_or(FIRST_FRAME_FALLBACK_DEADLINE)
}

/// Reads one complete length-prefixed wire frame (prefix and body) within
/// `deadline`, returning its raw bytes for replay.
///
/// `None` on timeout, EOF, transport fault, or an oversized length prefix —
/// in every case the caller simply drops the stream (the same silent teardown
/// `serve_one` applies to a bad first frame).
pub(crate) fn read_first_frame(
    recv: &mut RecvStream,
    handle: &Handle,
    deadline: Duration,
) -> Option<Vec<u8>> {
    handle.block_on(async move {
        tokio::time::timeout(deadline, async move {
            let mut frame = vec![0u8; 4];
            recv.read_exact(&mut frame[..]).await.ok()?;
            let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
            if len > wanix_mesh_wire::MAX_FRAME_LEN {
                return None;
            }
            frame.resize(4 + len, 0);
            recv.read_exact(&mut frame[4..]).await.ok()?;
            Some(frame)
        })
        .await
        .ok()
        .flatten()
    })
}

/// A sync stream that replays an already-read first frame, then continues on
/// the live stream. Writes always go to the live stream.
pub(crate) struct ReplayDuplex<S> {
    replay: io::Cursor<Vec<u8>>,
    live: S,
}

impl<S> ReplayDuplex<S> {
    pub(crate) fn new(first_frame: Vec<u8>, live: S) -> Self {
        Self {
            replay: io::Cursor::new(first_frame),
            live,
        }
    }
}

impl<S: Read> Read for ReplayDuplex<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let replayed = self.replay.read(buf)?;
        if replayed > 0 {
            return Ok(replayed);
        }
        self.live.read(buf)
    }
}

impl<S: Write> Write for ReplayDuplex<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.live.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.live.flush()
    }
}

/// The client side of one open-file stream: writes carry the per-op deadline;
/// reads carry it only until the first reply frame (the open response) is
/// complete, and are unbounded after (see the module docs).
pub(crate) struct OpenFileDuplex {
    recv: RecvStream,
    writer: BlockingWriter,
    handle: Handle,
    deadline: Option<Duration>,
    first_reply: FirstReply,
}

/// Progress through the first reply frame (4-byte LE length prefix + body).
enum FirstReply {
    Prefix { got: Vec<u8> },
    Body { remaining: u64 },
    Done,
}

impl OpenFileDuplex {
    pub(crate) fn new(
        send: SendStream,
        recv: RecvStream,
        handle: Handle,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            recv,
            writer: BlockingWriter::new(send, handle.clone(), deadline),
            handle,
            deadline,
            first_reply: FirstReply::Prefix { got: Vec::new() },
        }
    }

    /// Advances the first-reply state machine over `bytes` just delivered.
    fn advance(&mut self, mut bytes: &[u8]) {
        loop {
            match &mut self.first_reply {
                FirstReply::Prefix { got } => {
                    let take = (4 - got.len()).min(bytes.len());
                    got.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];
                    if got.len() < 4 {
                        return;
                    }
                    let len = u32::from_le_bytes([got[0], got[1], got[2], got[3]]) as u64;
                    if len == 0 {
                        self.first_reply = FirstReply::Done;
                        return;
                    }
                    self.first_reply = FirstReply::Body { remaining: len };
                }
                FirstReply::Body { remaining } => {
                    let take = (*remaining).min(bytes.len() as u64);
                    *remaining -= take;
                    if *remaining == 0 {
                        self.first_reply = FirstReply::Done;
                    }
                    return;
                }
                FirstReply::Done => return,
            }
        }
    }
}

impl Read for OpenFileDuplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let deadline = match self.first_reply {
            FirstReply::Done => None,
            _ => self.deadline,
        };
        let recv = &mut self.recv;
        let target = &mut *buf;
        let read = async move {
            recv.read(target)
                .await
                .map(|read| read.unwrap_or(0))
                .map_err(|err| io::Error::other(format!("quic recv read failed: {err}")))
        };
        let count = block_on_deadline(&self.handle, deadline, read)?;
        if count > 0 {
            self.advance(&buf[..count]);
        }
        Ok(count)
    }
}

impl Write for OpenFileDuplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::ReplayDuplex;

    /// A fake live stream: reads drain `input`, writes land in `output`.
    #[derive(Default)]
    struct FakeLive {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for FakeLive {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for FakeLive {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn replays_the_first_frame_then_continues_on_the_live_stream() {
        let live = FakeLive {
            input: std::io::Cursor::new(b"live-bytes".to_vec()),
            output: Vec::new(),
        };
        let mut duplex = ReplayDuplex::new(b"frame".to_vec(), live);

        // Short read buffers exercise the replay/live boundary.
        let mut buf = [0u8; 3];
        assert_eq!(duplex.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"fra");
        assert_eq!(duplex.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"me");
        assert_eq!(duplex.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"liv");
    }

    #[test]
    fn writes_pass_straight_through_to_the_live_stream() {
        let mut duplex = ReplayDuplex::new(b"unread-replay".to_vec(), FakeLive::default());
        duplex.write_all(b"reply").unwrap();
        assert_eq!(duplex.live.output, b"reply");
    }
}
