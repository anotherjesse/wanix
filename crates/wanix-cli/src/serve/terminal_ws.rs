use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use tungstenite::{Error as WsError, Message, WebSocket};
use wanix_fs::NormalizedPath;

use crate::p9_ws::P9WsConnectionError;
use crate::qjs_term::QjsShellSession;

use super::connection::ServeConnectionError;
use super::http::{HttpStatus, StaticResponse, percent_decode};

pub(super) const QJS_SHELL_WEBSOCKET_PATH: &str = "/.well-known/qjs-shell";
const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;

pub(super) fn is_qjs_shell_websocket_path(raw_path: Option<&str>) -> bool {
    raw_path.map(|path| path.split_once('?').map_or(path, |(path, _)| path))
        == Some(QJS_SHELL_WEBSOCKET_PATH)
}

pub(super) fn qjs_shell_cwd_from_target(
    raw_path: Option<&str>,
) -> Result<NormalizedPath, StaticResponse> {
    let raw_path =
        raw_path.ok_or_else(|| StaticResponse::plain(HttpStatus::BadRequest, "bad request"))?;
    let Some((_path, query)) = raw_path.split_once('?') else {
        return Ok(NormalizedPath::new(".").expect("default cwd is valid"));
    };
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = query_percent_decode(raw_key)?;
        if key == "cwd" {
            let value = query_percent_decode(raw_value)?;
            return qjs_shell_cwd_from_query_value(&value);
        }
    }
    Ok(NormalizedPath::new(".").expect("default cwd is valid"))
}

fn qjs_shell_cwd_from_query_value(value: &str) -> Result<NormalizedPath, StaticResponse> {
    let path = match value {
        "" => {
            return Err(StaticResponse::plain(
                HttpStatus::BadRequest,
                "invalid qjs shell cwd",
            ));
        }
        "/" => ".",
        path => path.strip_prefix('/').unwrap_or(path),
    };
    let path = if path.is_empty() { "." } else { path };
    NormalizedPath::new(path)
        .map_err(|_| StaticResponse::plain(HttpStatus::BadRequest, "invalid qjs shell cwd"))
}

fn query_percent_decode(value: &str) -> Result<String, StaticResponse> {
    percent_decode(&value.replace('+', " "))
        .map_err(|_| StaticResponse::plain(HttpStatus::BadRequest, "invalid query string"))
}

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

pub(super) fn parse_terminal_resize_message(text: &str) -> Option<(u16, u16)> {
    if let Some(resize) = parse_terminal_resize_json(text) {
        return Some(resize);
    }
    let mut parts = text.split_whitespace();
    if parts.next()? != "resize" {
        return None;
    }
    let columns = parts.next()?.parse().ok()?;
    let rows = parts.next()?.parse().ok()?;
    if parts.next().is_some() || columns == 0 || rows == 0 {
        return None;
    }
    Some((columns, rows))
}

fn parse_terminal_resize_json(text: &str) -> Option<(u16, u16)> {
    let compact = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if !compact.contains("\"type\":\"resize\"") {
        return None;
    }
    let columns = json_u16_field(&compact, "columns")?;
    let rows = json_u16_field(&compact, "rows")?;
    Some((columns, rows))
}

fn json_u16_field(compact_json: &str, field: &str) -> Option<u16> {
    let marker = format!("\"{field}\":");
    let start = compact_json.find(&marker)? + marker.len();
    let digits = compact_json[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let value = digits.parse().ok()?;
    (value != 0).then_some(value)
}
