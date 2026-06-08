use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use tungstenite::{Bytes, Error as WsError, Message, WebSocket};
use wanix_fs::NormalizedPath;

use crate::qjs_term::QjsShellSession;

use super::super::connection::ServeConnectionError;
use super::super::ws_duplex::WebSocketDoorError;
use super::message::parse_terminal_resize_message;

const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;

pub(in crate::serve) fn serve_terminal_websocket_connection(
    root_path: &Path,
    cwd: &NormalizedPath,
    socket: WebSocket<TcpStream>,
) -> Result<(), ServeConnectionError> {
    TerminalWebSocketSession::start(root_path, cwd, socket)?.run()
}

struct TerminalWebSocketSession {
    socket: WebSocket<TcpStream>,
    shell: QjsShellSession,
}

impl TerminalWebSocketSession {
    fn start(
        root_path: &Path,
        cwd: &NormalizedPath,
        mut socket: WebSocket<TcpStream>,
    ) -> Result<Self, ServeConnectionError> {
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(
                QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS,
            )))
            .map_err(ServeConnectionError::Io)?;
        let (shell, initial_output) = QjsShellSession::start_in_cwd(root_path, cwd)
            .map_err(ServeConnectionError::Terminal)?;
        let mut session = Self { socket, shell };
        session.send_output(initial_output)?;
        Ok(session)
    }

    fn run(&mut self) -> Result<(), ServeConnectionError> {
        loop {
            if self.handle_next_event()? {
                return Ok(());
            }
            if self.close_if_finished()? {
                return Ok(());
            }
        }
    }

    fn handle_next_event(&mut self) -> Result<bool, ServeConnectionError> {
        match self.read_event()? {
            TerminalWebSocketEvent::Message(message) => self.handle_message(message),
            TerminalWebSocketEvent::IdleTick => {
                self.pump_output()?;
                Ok(false)
            }
            TerminalWebSocketEvent::Closed => Ok(true),
        }
    }

    fn read_event(&mut self) -> Result<TerminalWebSocketEvent, ServeConnectionError> {
        match self.socket.read() {
            Ok(message) => Ok(TerminalWebSocketEvent::Message(message)),
            Err(WsError::ConnectionClosed) => Ok(TerminalWebSocketEvent::Closed),
            Err(error) if is_terminal_websocket_idle_tick(&error) => {
                Ok(TerminalWebSocketEvent::IdleTick)
            }
            Err(error) => Err(ws_error(error)),
        }
    }

    fn handle_message(&mut self, message: Message) -> Result<bool, ServeConnectionError> {
        match TerminalClientMessage::from(message) {
            TerminalClientMessage::Input(bytes) => self.feed_input(bytes.as_ref()).map(|_| false),
            TerminalClientMessage::Text(text) => self.handle_text(text.as_str()).map(|_| false),
            TerminalClientMessage::Close => self.close_requested(),
            TerminalClientMessage::Ping(bytes) => self.send_pong(bytes).map(|_| false),
            TerminalClientMessage::Ignore => Ok(false),
        }
    }

    fn handle_text(&mut self, text: &str) -> Result<(), ServeConnectionError> {
        if let Some((columns, rows)) = parse_terminal_resize_message(text) {
            return self.resize(columns, rows);
        }
        self.feed_input(text.as_bytes())
    }

    fn pump_output(&mut self) -> Result<(), ServeConnectionError> {
        let output = self.shell.pump().map_err(ServeConnectionError::Terminal)?;
        self.send_output(output)
    }

    fn feed_input(&mut self, input: &[u8]) -> Result<(), ServeConnectionError> {
        let output = self
            .shell
            .input(input)
            .map_err(ServeConnectionError::Terminal)?;
        self.send_output(output)
    }

    fn resize(&mut self, columns: u16, rows: u16) -> Result<(), ServeConnectionError> {
        let output = self
            .shell
            .resize(columns, rows)
            .map_err(ServeConnectionError::Terminal)?;
        self.send_output(output)
    }

    fn close_requested(&mut self) -> Result<bool, ServeConnectionError> {
        self.shell
            .close_terminal_resource()
            .map_err(ServeConnectionError::Terminal)?;
        Ok(true)
    }

    fn close_if_finished(&mut self) -> Result<bool, ServeConnectionError> {
        if !self.shell.is_finished() {
            return Ok(false);
        }
        let exit_code = self
            .shell
            .exit_code()
            .map_err(ServeConnectionError::Terminal)?
            .unwrap_or(0);
        self.send_exit(exit_code)?;
        self.shell
            .close_terminal_resource()
            .map_err(ServeConnectionError::Terminal)?;
        self.socket.close(None).map_err(ws_error)?;
        Ok(true)
    }

    fn send_output(&mut self, output: Vec<u8>) -> Result<(), ServeConnectionError> {
        if output.is_empty() {
            return Ok(());
        }
        self.socket.send(Message::binary(output)).map_err(ws_error)
    }

    fn send_pong(&mut self, bytes: Bytes) -> Result<(), ServeConnectionError> {
        self.socket.send(Message::Pong(bytes)).map_err(ws_error)
    }

    fn send_exit(&mut self, code: i32) -> Result<(), ServeConnectionError> {
        self.socket
            .send(Message::text(format!(
                "{{\"type\":\"exit\",\"code\":{code}}}"
            )))
            .map_err(ws_error)
    }
}

enum TerminalWebSocketEvent {
    Message(Message),
    IdleTick,
    Closed,
}

enum TerminalClientMessage {
    Input(Bytes),
    Text(String),
    Close,
    Ping(Bytes),
    Ignore,
}

impl From<Message> for TerminalClientMessage {
    fn from(message: Message) -> Self {
        match message {
            Message::Binary(bytes) => Self::Input(bytes),
            Message::Text(text) => Self::Text(text.to_string()),
            Message::Close(_) => Self::Close,
            Message::Ping(bytes) => Self::Ping(bytes),
            Message::Pong(_) | Message::Frame(_) => Self::Ignore,
        }
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

fn ws_error(error: WsError) -> ServeConnectionError {
    ServeConnectionError::WebSocket(WebSocketDoorError::Handshake(error.to_string()))
}
