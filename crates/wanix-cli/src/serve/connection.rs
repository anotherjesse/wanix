use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

use tungstenite::accept;
use wanix_9p::P9Server;

use crate::{CliError, write_process_output};

use super::ServeRoots;
use super::http::{
    HttpStatus, StaticResponse, is_websocket_upgrade, peek_request_headers, peek_request_target,
    serve_http_connection, websocket_rejection_response, write_static_response,
};
use super::terminal_ws::{
    is_qjs_shell_websocket_path, qjs_shell_cwd_from_target, serve_terminal_websocket_connection,
};
use super::ws_duplex::{WebSocketDoorError, WebSocketDuplex};

pub(super) fn serve_one_connection(
    listener: &TcpListener,
    roots: &ServeRoots,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("serve accept failed: {error}"), 1))?;
    match serve_connection(roots, stream, peer_addr) {
        Ok(()) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust serve: connection {peer_addr} failed: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}

pub(super) fn serve_connection(
    roots: &ServeRoots,
    stream: TcpStream,
    peer_addr: SocketAddr,
) -> Result<(), ServeConnectionError> {
    let request = peek_request_headers(&stream).map_err(ServeConnectionError::Io)?;
    if is_websocket_upgrade(&request) {
        return serve_websocket_request(roots, stream, peek_request_target(&request));
    }

    serve_http_connection(roots, stream, peer_addr)
}

fn serve_websocket_request(
    roots: &ServeRoots,
    stream: TcpStream,
    target: Option<&str>,
) -> Result<(), ServeConnectionError> {
    if is_qjs_shell_websocket_path(target) {
        if !roots.wanix_services {
            return write_static_response(
                stream,
                StaticResponse::plain(HttpStatus::NotFound, "not found"),
            );
        }
        let cwd = match qjs_shell_cwd_from_target(target) {
            Ok(cwd) => cwd,
            Err(response) => return write_static_response(stream, response),
        };
        let socket = accept_websocket(stream)?;
        return serve_terminal_websocket_connection(&roots.static_root, &cwd, socket);
    }
    if let Some(response) = websocket_rejection_response(target) {
        return write_static_response(stream, response);
    }
    let socket = accept_websocket(stream)?;
    let mut server = P9Server::new(Arc::clone(&roots.p9_root));
    server
        .serve_duplex(WebSocketDuplex::new(socket))
        .map(|_stats| ())
        .map_err(|error| ServeConnectionError::WebSocket(WebSocketDoorError::from(error)))
}

fn accept_websocket(
    stream: TcpStream,
) -> Result<tungstenite::WebSocket<TcpStream>, ServeConnectionError> {
    accept(stream).map_err(|error| {
        ServeConnectionError::WebSocket(WebSocketDoorError::Handshake(error.to_string()))
    })
}

#[derive(Debug)]
pub(super) enum ServeConnectionError {
    Io(io::Error),
    Http(String),
    WebSocket(WebSocketDoorError),
    Terminal(CliError),
}

impl fmt::Display for ServeConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O failed: {error}"),
            Self::Http(error) => f.write_str(error),
            Self::WebSocket(error) => write!(f, "websocket 9P failed: {error}"),
            Self::Terminal(error) => write!(f, "websocket terminal failed: {error}"),
        }
    }
}

impl Error for ServeConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::WebSocket(error) => Some(error),
            Self::Terminal(error) => Some(error),
            Self::Http(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io;

    use super::*;

    #[test]
    fn serve_connection_errors_have_stable_display_text() {
        assert_eq!(
            ServeConnectionError::Io(io::Error::other("closed pipe")).to_string(),
            "I/O failed: closed pipe"
        );
        assert_eq!(
            ServeConnectionError::Http("request headers exceeded limit".to_owned()).to_string(),
            "request headers exceeded limit"
        );
        assert_eq!(
            ServeConnectionError::WebSocket(WebSocketDoorError::Handshake("bad key".to_owned()))
                .to_string(),
            "websocket 9P failed: websocket handshake failed: bad key"
        );
        assert_eq!(
            ServeConnectionError::Terminal(CliError::new("shell failed", 1)).to_string(),
            "websocket terminal failed: shell failed"
        );
    }

    #[test]
    fn serve_connection_errors_report_sources_for_wrapped_errors() {
        let io_error = ServeConnectionError::Io(io::Error::other("closed pipe"));
        assert!(io_error.source().unwrap().is::<io::Error>());

        let websocket_error =
            ServeConnectionError::WebSocket(WebSocketDoorError::Handshake("bad key".to_owned()));
        assert!(websocket_error.source().unwrap().is::<WebSocketDoorError>());

        let terminal_error = ServeConnectionError::Terminal(CliError::new("shell failed", 1));
        assert!(terminal_error.source().unwrap().is::<CliError>());

        let http_error = ServeConnectionError::Http("bad request".to_owned());
        assert!(http_error.source().is_none());
    }
}
