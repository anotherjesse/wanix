use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;

use tungstenite::{Error as WsError, Message, WebSocket};
use wanix_9p::P9TransportError;

/// Adapts a tungstenite [`WebSocket`] into a plain [`Read`] + [`Write`] byte
/// stream so the single 9P session loop ([`wanix_9p::P9Server::serve_duplex`])
/// drives the websocket door with no second server implementation.
///
/// HTTP upgrade happens before construction; this type owns ws de/masking and
/// reassembly. 9P frame splitting across ws messages is handled by the session
/// core, so `read_buf` only holds one ws message's leftover bytes between reads.
pub(crate) struct WebSocketDuplex {
    socket: WebSocket<TcpStream>,
    read_buf: VecDeque<u8>,
}

impl WebSocketDuplex {
    pub(crate) fn new(socket: WebSocket<TcpStream>) -> Self {
        Self {
            socket,
            read_buf: VecDeque::new(),
        }
    }
}

impl Read for WebSocketDuplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.read_buf.is_empty() {
                let count = self.read_buf.len().min(buf.len());
                for slot in buf.iter_mut().take(count) {
                    *slot = self.read_buf.pop_front().expect("buffered byte");
                }
                return Ok(count);
            }

            match self.socket.read() {
                Ok(Message::Binary(bytes)) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    self.read_buf.extend(bytes.iter().copied());
                }
                Ok(Message::Ping(bytes)) => {
                    // Answer keepalives inside read() so the session core never
                    // sees control frames.
                    self.socket
                        .send(Message::Pong(bytes))
                        .map_err(ws_to_io_error)?;
                }
                Ok(Message::Close(_)) => return Ok(0),
                Ok(Message::Text(_) | Message::Pong(_) | Message::Frame(_)) => continue,
                Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(0),
                Err(error) => return Err(ws_to_io_error(error)),
            }
        }
    }
}

impl Write for WebSocketDuplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // serve_duplex writes one response frame per write_all, so one response
        // frame maps to exactly one binary ws message.
        self.socket
            .send(Message::binary(buf.to_vec()))
            .map_err(ws_to_io_error)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.socket.flush() {
            Ok(()) | Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => Ok(()),
            Err(error) => Err(ws_to_io_error(error)),
        }
    }
}

fn ws_to_io_error(error: WsError) -> io::Error {
    match error {
        WsError::Io(io_error) => io_error,
        other => io::Error::other(other),
    }
}

/// Error from the serve websocket 9P door: either the HTTP/websocket handshake
/// or the per-connection 9P session over the [`WebSocketDuplex`].
#[derive(Debug)]
pub(crate) enum WebSocketDoorError {
    Handshake(String),
    Transport(P9TransportError),
}

impl fmt::Display for WebSocketDoorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake(error) => write!(f, "websocket handshake failed: {error}"),
            Self::Transport(error) => write!(f, "websocket 9P session failed: {error}"),
        }
    }
}

impl Error for WebSocketDoorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Handshake(_) => None,
            Self::Transport(error) => Some(error),
        }
    }
}

impl From<P9TransportError> for WebSocketDoorError {
    fn from(error: P9TransportError) -> Self {
        Self::Transport(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use tungstenite::{Message, WebSocket, accept, connect};
    use wanix_9p::P9Server;
    use wanix_fs::{FileSystem, MemFs};
    use wanix_protocol::{
        P9_RATTACH, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L, P9Frame,
        P9FrameBuffer, p9_decode_rread, p9_tattach, p9_tlopen, p9_tread, p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn door_error_display_and_source_match_the_doors() {
        let handshake = WebSocketDoorError::Handshake("bad key".to_owned());
        assert_eq!(handshake.to_string(), "websocket handshake failed: bad key");
        assert!(handshake.source().is_none());

        let transport =
            WebSocketDoorError::Transport(P9TransportError::TruncatedFrame { buffered_len: 7 });
        assert_eq!(
            transport.to_string(),
            "websocket 9P session failed: 9P transport EOF with 7 buffered bytes"
        );
        assert!(transport.source().is_some());
    }

    #[test]
    fn websocket_duplex_serves_a_session_over_the_core() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello ws").unwrap();
        let root: Arc<dyn FileSystem> = fs;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = thread::spawn(move || {
            let (stream, _peer) = listener.accept().unwrap();
            let socket = accept(stream).unwrap();
            let mut server = P9Server::new(root);
            server.serve_duplex(WebSocketDuplex::new(socket)).unwrap()
        });

        let mut socket = connect(format!("ws://{addr}/")).unwrap().0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 8),
        ]);
        socket.send(Message::binary(requests)).unwrap();
        let frames = read_binary_frames(&mut socket, 5);
        socket.close(None).unwrap();
        // Drain the close handshake so the server's read() observes the close.
        while socket.read().is_ok() {}

        let stats = server_thread.join().unwrap();
        assert_eq!(stats.requests, 5);
        assert_eq!(stats.responses, 5);
        assert_eq!(
            frame_types(&frames),
            [P9_RVERSION, P9_RATTACH, P9_RWALK, P9_RLOPEN, P9_RREAD]
        );
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello ws");
    }

    #[test]
    fn websocket_duplex_reassembles_a_frame_split_across_ws_messages() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello ws").unwrap();
        let root: Arc<dyn FileSystem> = fs;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = thread::spawn(move || {
            let (stream, _peer) = listener.accept().unwrap();
            let socket = accept(stream).unwrap();
            let mut server = P9Server::new(root);
            server.serve_duplex(WebSocketDuplex::new(socket)).unwrap()
        });

        let mut socket = connect(format!("ws://{addr}/")).unwrap().0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 8),
        ]);
        // Send one byte per ws message to force cross-message frame reassembly
        // in the session core (the duplex only holds one message's leftovers).
        for byte in &requests {
            socket.send(Message::binary(vec![*byte])).unwrap();
        }
        let frames = read_binary_frames(&mut socket, 5);
        socket.close(None).unwrap();
        while socket.read().is_ok() {}

        let stats = server_thread.join().unwrap();
        assert_eq!(stats.requests, 5);
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello ws");
        assert_eq!(frame_types(&frames)[0], P9_RVERSION);
    }

    fn read_binary_frames<S: std::io::Read + std::io::Write>(
        socket: &mut WebSocket<S>,
        count: usize,
    ) -> Vec<P9Frame> {
        let mut buffer = P9FrameBuffer::new();
        let mut frames = Vec::new();
        while frames.len() < count {
            if let Message::Binary(bytes) = socket.read().unwrap() {
                frames.extend(buffer.push(&bytes).unwrap());
            }
        }
        frames
    }

    fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut stream = Vec::new();
        for frame in frames {
            stream.extend_from_slice(&frame.encode().unwrap());
        }
        stream
    }

    fn frame_types(frames: &[P9Frame]) -> Vec<u8> {
        frames.iter().map(P9Frame::message_type).collect()
    }
}
