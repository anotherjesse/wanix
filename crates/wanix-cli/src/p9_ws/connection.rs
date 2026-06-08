use std::error::Error;
use std::fmt;
use std::net::TcpStream;
use std::sync::Arc;

use tungstenite::WebSocket;
use wanix_9p::{P9Server, P9TransportError};
use wanix_fs::FileSystem;

use crate::serve::WebSocketDuplex;

/// Error from the standalone `p9-ws` door. The per-connection 9P session is run
/// by the single core ([`P9Server::serve_duplex`]) over the shared
/// [`WebSocketDuplex`] adapter; this enum only distinguishes the handshake from
/// a session/transport failure. (Retired alongside `p9-ws` itself.)
#[derive(Debug)]
pub(crate) enum P9WsConnectionError {
    Handshake(String),
    Transport(P9TransportError),
}

impl fmt::Display for P9WsConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake(error) => write!(f, "websocket handshake failed: {error}"),
            Self::Transport(error) => write!(f, "websocket 9P session failed: {error}"),
        }
    }
}

impl Error for P9WsConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Handshake(_) => None,
            Self::Transport(error) => Some(error),
        }
    }
}

impl From<P9TransportError> for P9WsConnectionError {
    fn from(error: P9TransportError) -> Self {
        Self::Transport(error)
    }
}

/// Serves one 9P websocket connection by handing the socket to the single
/// session core as a [`WebSocketDuplex`] byte stream. There is no second 9P
/// loop here.
pub(crate) fn serve_websocket_connection(
    root: Arc<dyn FileSystem>,
    socket: WebSocket<TcpStream>,
) -> Result<(), P9WsConnectionError> {
    let mut server = P9Server::new(root);
    server
        .serve_duplex(WebSocketDuplex::new(socket))
        .map(|_stats| ())
        .map_err(P9WsConnectionError::from)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn p9_ws_connection_errors_have_stable_display_text() {
        assert_eq!(
            P9WsConnectionError::Handshake("bad key".to_owned()).to_string(),
            "websocket handshake failed: bad key"
        );
        assert_eq!(
            P9WsConnectionError::Transport(P9TransportError::TruncatedFrame { buffered_len: 7 })
                .to_string(),
            "websocket 9P session failed: 9P transport EOF with 7 buffered bytes"
        );
    }

    #[test]
    fn p9_ws_connection_errors_report_sources_for_wrapped_errors() {
        assert!(Error::source(&P9WsConnectionError::Handshake("bad key".to_owned())).is_none());
        assert!(
            Error::source(&P9WsConnectionError::Transport(
                P9TransportError::TruncatedFrame { buffered_len: 7 }
            ))
            .is_some()
        );
    }
}
