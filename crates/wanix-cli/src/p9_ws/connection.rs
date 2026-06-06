use std::error::Error;
use std::fmt;
use std::net::TcpStream;
use std::sync::Arc;

use tungstenite::{Error as WsError, Message, WebSocket};
use wanix_9p::P9Server;
use wanix_fs::FileSystem;
use wanix_protocol::{P9Error, P9FrameBuffer};

#[derive(Debug)]
pub(crate) enum P9WsConnectionError {
    Handshake(String),
    WebSocket(WsError),
    Protocol(P9Error),
    Server(wanix_9p::Wanix9pError),
    TruncatedFrame { buffered_len: usize },
}

impl fmt::Display for P9WsConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake(error) => write!(f, "websocket handshake failed: {error}"),
            Self::WebSocket(error) => write!(f, "websocket I/O failed: {error}"),
            Self::Protocol(error) => write!(f, "9P protocol error: {error}"),
            Self::Server(error) => write!(f, "9P server error: {error}"),
            Self::TruncatedFrame { buffered_len } => {
                write!(f, "websocket closed with {buffered_len} buffered 9P bytes")
            }
        }
    }
}

impl Error for P9WsConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WebSocket(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Server(error) => Some(error),
            Self::Handshake(_) | Self::TruncatedFrame { .. } => None,
        }
    }
}

macro_rules! impl_connection_error_from {
    ($source:ty => $variant:ident) => {
        impl From<$source> for P9WsConnectionError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}

impl_connection_error_from!(WsError => WebSocket);
impl_connection_error_from!(P9Error => Protocol);
impl_connection_error_from!(wanix_9p::Wanix9pError => Server);

pub(crate) fn serve_websocket_connection(
    root: Arc<dyn FileSystem>,
    mut socket: WebSocket<TcpStream>,
) -> Result<(), P9WsConnectionError> {
    let mut server = P9Server::new(root);
    let mut frames = P9FrameBuffer::new();

    loop {
        let Some(message) = read_websocket_message(&mut socket, frames.buffered_len())? else {
            return Ok(());
        };
        if !handle_websocket_message(&mut server, &mut frames, &mut socket, message)? {
            return Ok(());
        }
    }
}

fn read_websocket_message(
    socket: &mut WebSocket<TcpStream>,
    buffered_len: usize,
) -> Result<Option<Message>, P9WsConnectionError> {
    match socket.read() {
        Ok(message) => Ok(Some(message)),
        Err(WsError::ConnectionClosed) if buffered_len == 0 => Ok(None),
        Err(WsError::ConnectionClosed) => Err(P9WsConnectionError::TruncatedFrame { buffered_len }),
        Err(error) => Err(P9WsConnectionError::WebSocket(error)),
    }
}

fn handle_websocket_message(
    server: &mut P9Server,
    frames: &mut P9FrameBuffer,
    socket: &mut WebSocket<TcpStream>,
    message: Message,
) -> Result<bool, P9WsConnectionError> {
    match message {
        Message::Binary(bytes) => handle_binary_websocket_message(server, frames, socket, &bytes),
        Message::Close(_) => close_websocket_connection(frames),
        Message::Ping(bytes) => {
            socket.send(Message::Pong(bytes))?;
            Ok(true)
        }
        Message::Text(_) | Message::Pong(_) | Message::Frame(_) => Ok(true),
    }
}

fn handle_binary_websocket_message(
    server: &mut P9Server,
    frames: &mut P9FrameBuffer,
    socket: &mut WebSocket<TcpStream>,
    bytes: &[u8],
) -> Result<bool, P9WsConnectionError> {
    for request in frames.push(bytes)? {
        let response = server.handle_frame(&request)?;
        let response_bytes = response.encode()?;
        socket.send(Message::binary(response_bytes))?;
    }
    Ok(true)
}

fn close_websocket_connection(frames: &P9FrameBuffer) -> Result<bool, P9WsConnectionError> {
    if frames.buffered_len() != 0 {
        return Err(P9WsConnectionError::TruncatedFrame {
            buffered_len: frames.buffered_len(),
        });
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io;

    use super::*;

    #[test]
    fn p9_ws_connection_errors_have_stable_display_text() {
        assert_eq!(
            P9WsConnectionError::Handshake("bad key".to_owned()).to_string(),
            "websocket handshake failed: bad key"
        );
        assert!(
            P9WsConnectionError::WebSocket(WsError::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pipe closed"
            )))
            .to_string()
            .starts_with("websocket I/O failed:"),
        );
        assert_eq!(
            P9WsConnectionError::Protocol(P9Error::InvalidUtf8).to_string(),
            "9P protocol error: 9P string is not valid UTF-8"
        );
        assert_eq!(
            P9WsConnectionError::Server(wanix_9p::Wanix9pError::InvalidPath("../x".to_owned()))
                .to_string(),
            "9P server error: invalid 9P walk path: ../x"
        );
        assert_eq!(
            P9WsConnectionError::TruncatedFrame { buffered_len: 7 }.to_string(),
            "websocket closed with 7 buffered 9P bytes"
        );
    }

    #[test]
    fn p9_ws_connection_errors_report_sources_for_wrapped_errors() {
        assert!(Error::source(&P9WsConnectionError::Handshake("bad key".to_owned())).is_none());
        assert!(
            Error::source(&P9WsConnectionError::WebSocket(WsError::Io(
                io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed")
            )))
            .is_some()
        );
        assert!(Error::source(&P9WsConnectionError::Protocol(P9Error::InvalidUtf8)).is_some());
        assert!(
            Error::source(&P9WsConnectionError::Server(
                wanix_9p::Wanix9pError::InvalidPath("../x".to_owned())
            ))
            .is_some()
        );
        assert!(Error::source(&P9WsConnectionError::TruncatedFrame { buffered_len: 7 }).is_none());
    }
}
