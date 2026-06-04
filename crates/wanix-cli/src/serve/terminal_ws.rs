use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use tungstenite::{Error as WsError, Message, WebSocket};
use wanix_fs::NormalizedPath;

use crate::p9_ws::P9WsConnectionError;
use crate::qjs_term::QjsShellSession;

use super::connection::ServeConnectionError;

mod message;
mod request;

pub(super) use message::parse_terminal_resize_message;
pub(super) use request::{
    QJS_SHELL_WEBSOCKET_PATH, is_qjs_shell_websocket_path, qjs_shell_cwd_from_target,
};

const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;

pub(super) fn serve_terminal_websocket_connection(
    root_path: &Path,
    cwd: &NormalizedPath,
    mut socket: WebSocket<TcpStream>,
) -> Result<(), ServeConnectionError> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(
            QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS,
        )))?;
    let (mut session, initial_output) =
        QjsShellSession::start_in_cwd(root_path, cwd).map_err(ServeConnectionError::Terminal)?;
    send_terminal_output(&mut socket, initial_output)?;
    loop {
        let message = match socket.read() {
            Ok(message) => message,
            Err(WsError::ConnectionClosed) => return Ok(()),
            Err(error) if is_terminal_websocket_idle_tick(&error) => {
                let output = session.pump().map_err(ServeConnectionError::Terminal)?;
                send_terminal_output(&mut socket, output)?;
                if close_terminal_websocket_if_finished(&mut socket, &mut session)? {
                    return Ok(());
                }
                continue;
            }
            Err(error) => {
                return Err(ServeConnectionError::WebSocket(
                    P9WsConnectionError::WebSocket(error),
                ));
            }
        };
        if handle_terminal_websocket_message(&mut socket, &mut session, message)? {
            return Ok(());
        }
        if close_terminal_websocket_if_finished(&mut socket, &mut session)? {
            return Ok(());
        }
    }
}

fn handle_terminal_websocket_message(
    socket: &mut WebSocket<TcpStream>,
    session: &mut QjsShellSession,
    message: Message,
) -> Result<bool, ServeConnectionError> {
    match message {
        Message::Binary(bytes) => {
            let output = session
                .input(bytes.as_ref())
                .map_err(ServeConnectionError::Terminal)?;
            send_terminal_output(socket, output)?;
            Ok(false)
        }
        Message::Text(text) => {
            let text = text.as_str();
            let output = if let Some((columns, rows)) = parse_terminal_resize_message(text) {
                session
                    .resize(columns, rows)
                    .map_err(ServeConnectionError::Terminal)?
            } else {
                session
                    .input(text.as_bytes())
                    .map_err(ServeConnectionError::Terminal)?
            };
            send_terminal_output(socket, output)?;
            Ok(false)
        }
        Message::Close(_) => {
            session
                .close_terminal_resource()
                .map_err(ServeConnectionError::Terminal)?;
            Ok(true)
        }
        Message::Ping(bytes) => socket
            .send(Message::Pong(bytes))
            .map_err(|error| ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error)))
            .map(|_| false),
        Message::Pong(_) | Message::Frame(_) => Ok(false),
    }
}

fn is_terminal_websocket_idle_tick(error: &WsError) -> bool {
    matches!(
        error,
        WsError::Io(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )
    )
}

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

fn send_terminal_output(
    socket: &mut WebSocket<TcpStream>,
    output: Vec<u8>,
) -> Result<(), ServeConnectionError> {
    if output.is_empty() {
        return Ok(());
    }
    socket
        .send(Message::binary(output))
        .map_err(|error| ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error)))
}

fn send_terminal_exit(
    socket: &mut WebSocket<TcpStream>,
    code: i32,
) -> Result<(), ServeConnectionError> {
    socket
        .send(Message::text(format!(
            "{{\"type\":\"exit\",\"code\":{code}}}"
        )))
        .map_err(|error| ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error)))
}
