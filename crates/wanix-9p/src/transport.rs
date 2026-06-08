use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};

use wanix_protocol::{P9Error, P9FrameBuffer};

use crate::{P9Server, Wanix9pError};

const STREAM_READ_BUFFER_BYTES: usize = 8192;

/// Summary returned after a 9P byte stream reaches EOF cleanly.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct P9TransportStats {
    /// Number of request frames decoded and passed to the server.
    pub requests: usize,
    /// Number of response frames encoded and written.
    pub responses: usize,
    /// Number of bytes read from the transport.
    pub bytes_in: usize,
    /// Number of response bytes written to the transport.
    pub bytes_out: usize,
}

/// Transport-level error for sync 9P stream serving.
#[derive(Debug)]
pub enum P9TransportError {
    /// The underlying reader or writer failed.
    Io(io::Error),
    /// The byte stream failed 9P frame splitting or response encoding.
    Protocol(P9Error),
    /// The server rejected a decoded request too malformed for a 9P reply.
    Server(Wanix9pError),
    /// EOF arrived before the final request frame was complete.
    TruncatedFrame {
        /// Bytes buffered when EOF arrived.
        buffered_len: usize,
    },
}

impl fmt::Display for P9TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "9P transport I/O error: {error}"),
            Self::Protocol(error) => write!(f, "9P transport protocol error: {error}"),
            Self::Server(error) => write!(f, "9P server error: {error}"),
            Self::TruncatedFrame { buffered_len } => {
                write!(f, "9P transport EOF with {buffered_len} buffered bytes")
            }
        }
    }
}

impl Error for P9TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Server(error) => Some(error),
            Self::TruncatedFrame { .. } => None,
        }
    }
}

impl From<io::Error> for P9TransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<P9Error> for P9TransportError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

impl From<Wanix9pError> for P9TransportError {
    fn from(error: Wanix9pError) -> Self {
        Self::Server(error)
    }
}

/// Pairs an independent `Read` and `Write` half into one `Read + Write` duplex
/// so the single 9P session loop in [`P9Server::serve_duplex`] can drive both
/// the split-stream ([`P9Server::serve_stream`]) and single-owned-duplex
/// (websocket) transports through one body.
struct DuplexPair<R, W> {
    reader: R,
    writer: W,
}

impl<R: Read, W: Write> Read for DuplexPair<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<R: Read, W: Write> Write for DuplexPair<R, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl P9Server {
    /// Serves decoded 9P request frames from `reader` and writes response
    /// frames to `writer` until EOF.
    ///
    /// This is the split-stream entry point (separate read/write halves, e.g.
    /// a `TcpStream` and its `try_clone`, or a pipe pair). It shares the one
    /// per-connection session loop with [`P9Server::serve_duplex`] by wrapping
    /// the halves in an internal duplex; the loop body lives in exactly one
    /// place.
    ///
    /// Filesystem and unsupported-operation failures remain ordinary 9P
    /// response frames. Frame stream failures and malformed typed request
    /// payloads return transport errors because a real connection should stop.
    ///
    /// # Errors
    ///
    /// Returns an error when I/O fails, frame splitting/encoding fails, a
    /// decoded request is too malformed for a 9P response, or EOF arrives with
    /// a partial request frame buffered.
    pub fn serve_stream<R: Read, W: Write>(
        &mut self,
        reader: R,
        writer: W,
    ) -> Result<P9TransportStats, P9TransportError> {
        self.serve_duplex(DuplexPair { reader, writer })
    }

    /// Serves a 9P session over a single owned bidirectional byte stream.
    ///
    /// This is the canonical per-connection session loop; [`P9Server::serve_stream`]
    /// delegates to it over an internal duplex. Use this directly when the
    /// transport is one owned object that cannot be split into independent
    /// read/write halves (e.g. a websocket adapter). The loop never reads and
    /// writes concurrently, so a single `&mut D` is sufficient.
    ///
    /// # Errors
    ///
    /// Returns an error when I/O fails, frame splitting/encoding fails, a
    /// decoded request is too malformed for a 9P response, or EOF arrives with
    /// a partial request frame buffered.
    pub fn serve_duplex<D: Read + Write>(
        &mut self,
        mut duplex: D,
    ) -> Result<P9TransportStats, P9TransportError> {
        let mut frames = P9FrameBuffer::new();
        let mut stats = P9TransportStats::default();
        let mut buf = [0_u8; STREAM_READ_BUFFER_BYTES];

        loop {
            let count = duplex.read(&mut buf)?;
            if count == 0 {
                if frames.buffered_len() != 0 {
                    return Err(P9TransportError::TruncatedFrame {
                        buffered_len: frames.buffered_len(),
                    });
                }
                duplex.flush()?;
                return Ok(stats);
            }

            stats.bytes_in += count;
            for request in frames.push(&buf[..count])? {
                stats.requests += 1;
                let response = self.handle_frame(&request)?;
                let response_bytes = response.encode()?;
                duplex.write_all(&response_bytes)?;
                stats.bytes_out += response_bytes.len();
                stats.responses += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs};
    use wanix_protocol::{
        P9_RATTACH, P9_RLOPEN, P9_RREAD, P9_RREADDIR, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L,
        P9Frame, P9FrameBuffer, p9_decode_rread, p9_decode_rreaddir, p9_tattach, p9_tlopen,
        p9_tread, p9_treaddir, p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn serve_stream_handles_multiple_partial_requests_and_writes_responses() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello 9p").unwrap();
        let mut server = server(fs);
        let input = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 5),
        ]);
        let mut output = Vec::new();

        let stats = server
            .serve_stream(SlowReader::new(input.clone(), 3), &mut output)
            .unwrap();

        assert_eq!(stats.requests, 5);
        assert_eq!(stats.responses, 5);
        assert_eq!(stats.bytes_in, input.len());
        assert_eq!(stats.bytes_out, output.len());
        let frames = decode_response_stream(&output);
        assert_eq!(
            frame_types(&frames),
            [P9_RVERSION, P9_RATTACH, P9_RWALK, P9_RLOPEN, P9_RREAD]
        );
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello");
    }

    #[test]
    fn serve_stream_can_browse_directory_entries() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("bin").unwrap();
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);
        let input = request_stream([
            p9_tversion(1, 4096, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_tlopen(3, 1, 0),
            p9_treaddir(4, 1, 0, 4096),
        ]);
        let mut output = Vec::new();

        let stats = server.serve_stream(input.as_slice(), &mut output).unwrap();

        assert_eq!(stats.requests, 4);
        let frames = decode_response_stream(&output);
        assert_eq!(
            frame_types(&frames),
            [P9_RVERSION, P9_RATTACH, P9_RLOPEN, P9_RREADDIR]
        );
        let entries = p9_decode_rreaddir(&frames[3]).unwrap();
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["bin", "hello.txt"]);
    }

    #[test]
    fn serve_stream_reports_truncated_final_frame() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(fs);
        let mut input = p9_tversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        input.truncate(input.len() - 2);
        let mut output = Vec::new();

        let error = server
            .serve_stream(input.as_slice(), &mut output)
            .unwrap_err();

        assert!(matches!(
            error,
            P9TransportError::TruncatedFrame { buffered_len } if buffered_len > 0
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn serve_duplex_round_trips_over_one_byte_stream() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello 9p").unwrap();
        let mut server = server(fs);
        let input = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 5),
        ]);
        let mut duplex = DuplexBuffer::new(input);

        let stats = server.serve_duplex(&mut duplex).unwrap();

        assert_eq!(stats.requests, 5);
        assert_eq!(stats.responses, 5);
        let frames = decode_response_stream(&duplex.written);
        assert_eq!(
            frame_types(&frames),
            [P9_RVERSION, P9_RATTACH, P9_RWALK, P9_RLOPEN, P9_RREAD]
        );
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello");
    }

    #[test]
    fn transport_error_sources_preserve_wrapped_errors() {
        use std::error::Error as _;

        let io_error = P9TransportError::Io(io::Error::other("closed pipe"));
        assert!(io_error.source().unwrap().is::<io::Error>());

        let protocol_error = P9TransportError::Protocol(P9Error::InvalidFrameSize { size: 4 });
        assert!(protocol_error.source().unwrap().is::<P9Error>());

        let server_error =
            P9TransportError::Server(Wanix9pError::InvalidPath("../escape".to_owned()));
        assert!(server_error.source().unwrap().is::<Wanix9pError>());

        let truncated = P9TransportError::TruncatedFrame { buffered_len: 4 };
        assert!(truncated.source().is_none());
    }

    fn server(fs: Arc<MemFs>) -> P9Server {
        let root: Arc<dyn FileSystem> = fs;
        P9Server::new(root)
    }

    fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut stream = Vec::new();
        for frame in frames {
            stream.extend_from_slice(&frame.encode().unwrap());
        }
        stream
    }

    fn decode_response_stream(bytes: &[u8]) -> Vec<P9Frame> {
        let mut buffer = P9FrameBuffer::new();
        let frames = buffer.push(bytes).unwrap();
        assert_eq!(buffer.buffered_len(), 0);
        frames
    }

    fn frame_types(frames: &[P9Frame]) -> Vec<u8> {
        frames.iter().map(P9Frame::message_type).collect()
    }

    /// In-memory `Read + Write` duplex: reads drain `to_read`, writes append to
    /// `written`. Proves the single owned-duplex session path.
    struct DuplexBuffer {
        to_read: Vec<u8>,
        offset: usize,
        written: Vec<u8>,
    }

    impl DuplexBuffer {
        fn new(to_read: Vec<u8>) -> Self {
            Self {
                to_read,
                offset: 0,
                written: Vec::new(),
            }
        }
    }

    impl Read for DuplexBuffer {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = (self.to_read.len() - self.offset).min(buf.len());
            buf[..count].copy_from_slice(&self.to_read[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }
    }

    impl Write for DuplexBuffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct SlowReader {
        bytes: Vec<u8>,
        chunk_len: usize,
        offset: usize,
    }

    impl SlowReader {
        fn new(bytes: Vec<u8>, chunk_len: usize) -> Self {
            Self {
                bytes,
                chunk_len,
                offset: 0,
            }
        }
    }

    impl Read for SlowReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.offset >= self.bytes.len() {
                return Ok(0);
            }
            let count = self
                .chunk_len
                .min(buf.len())
                .min(self.bytes.len() - self.offset);
            buf[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }
    }
}
