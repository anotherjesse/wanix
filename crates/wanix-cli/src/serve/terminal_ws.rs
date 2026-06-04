use std::net::TcpStream;

use tungstenite::WebSocket;

use crate::p9_ws::P9WsConnectionError;
use crate::qjs_term::QjsShellSession;

use super::{ServeConnectionError, send_terminal_exit};

pub(super) fn close_terminal_websocket_if_finished(
    socket: &mut WebSocket<TcpStream>,
    session: &mut QjsShellSession,
) -> Result<bool, ServeConnectionError> {
    if !session.is_finished() {
        return Ok(false);
    }
    let exit_code = session
        .exit_code()
        .map_err(ServeConnectionError::Terminal)?
        .unwrap_or(0);
    send_terminal_exit(socket, exit_code)?;
    session
        .close_terminal_resource()
        .map_err(ServeConnectionError::Terminal)?;
    socket
        .close(None)
        .map_err(|error| ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error)))?;
    Ok(true)
}
