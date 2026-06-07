use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use tungstenite::{Bytes, Error as WsError, Message, WebSocket};
use wanix_fs::NormalizedPath;

use crate::p9_ws::P9WsConnectionError;
use crate::qjs_term::QjsShellSession;

use super::super::connection::ServeConnectionError;
use super::command_record::{ShellCommandRecord, strip_shell_command_records};
use super::message::parse_terminal_resize_message;
use super::protocol::{shell_exit_message, shell_mutation_message, shell_session_message};
use super::root_changes::RootChangeTracker;
use super::shell_activity::{
    ShellInputActivityTracker, ShellMutationOperation, operations_for_changed_paths,
    operations_without_changed_paths,
};
use super::shell_history::ShellHistoryWriter;
use super::shell_observation::{ShellCommandObservation, ShellCommandObservations};

const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;
const SHELL_OPERATION_DIAGNOSTIC_MAX_CHARS: usize = 240;
const SHELL_OPERATION_OUTPUT_MAX_CHARS: usize = 320;

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
    changes: RootChangeTracker,
    input: ShellInputActivityTracker,
    history: ShellHistoryWriter,
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
        let (shell, initial_output) =
            QjsShellSession::start_in_cwd_with_command_records(root_path, cwd, true)
                .map_err(ServeConnectionError::Terminal)?;
        let changes = RootChangeTracker::new(root_path).map_err(ServeConnectionError::Io)?;
        let mut session = Self {
            socket,
            input: ShellInputActivityTracker::new(cwd),
            shell,
            changes,
            history: ShellHistoryWriter::new(root_path),
        };
        session.send_session()?;
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
        let operations = self.input.observe_input(input);
        let output = self
            .shell
            .input(input)
            .map_err(ServeConnectionError::Terminal)?;
        let (output, records) = strip_shell_command_records(output);
        let observations = if input_contains_line_boundary(input) {
            shell_command_observations(input, &output, records)
        } else {
            ShellCommandObservations::default()
        };
        self.send_output(output)?;
        if input_contains_line_boundary(input) {
            self.send_mutations(&operations, &observations)?;
            self.input.sync_cwd(&self.shell.cwd());
        }
        Ok(())
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
        let (output, records) = strip_shell_command_records(output);
        self.write_record_history(
            &ShellCommandObservations::new(records, ShellCommandObservation::default()),
            &[],
        )?;
        if output.is_empty() {
            return Ok(());
        }
        self.socket.send(Message::binary(output)).map_err(ws_error)
    }

    fn send_session(&mut self) -> Result<(), ServeConnectionError> {
        self.socket
            .send(Message::text(shell_session_message(&self.shell)))
            .map_err(ws_error)
    }

    fn send_mutations(
        &mut self,
        operations: &[ShellMutationOperation],
        observations: &ShellCommandObservations,
    ) -> Result<(), ServeConnectionError> {
        let paths = self
            .changes
            .take_changed_paths()
            .map_err(ServeConnectionError::Io)?;
        let operations = if paths.is_empty() {
            operations_without_changed_paths(operations, observations)
        } else {
            operations_for_changed_paths(operations, &paths, observations)
        };
        let history_paths = self.write_record_history(observations, &operations)?;
        if paths.is_empty() && operations.is_empty() {
            return Ok(());
        }
        self.socket
            .send(Message::text(shell_mutation_message(
                &self.shell,
                &paths,
                &operations,
                &history_paths,
            )))
            .map_err(ws_error)
    }

    fn write_record_history(
        &mut self,
        observations: &ShellCommandObservations,
        operations: &[ShellMutationOperation],
    ) -> Result<Vec<String>, ServeConnectionError> {
        let paths = self
            .history
            .write_from_shell(&self.shell, observations.recorded_commands(), operations)
            .map_err(ServeConnectionError::Io)?;
        if !paths.is_empty() {
            self.changes
                .take_changed_paths()
                .map_err(ServeConnectionError::Io)?;
        }
        Ok(paths)
    }

    fn send_pong(&mut self, bytes: Bytes) -> Result<(), ServeConnectionError> {
        self.socket.send(Message::Pong(bytes)).map_err(ws_error)
    }

    fn send_exit(&mut self, code: i32) -> Result<(), ServeConnectionError> {
        self.socket
            .send(Message::text(shell_exit_message(code)))
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
    ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error))
}

fn input_contains_line_boundary(input: &[u8]) -> bool {
    input.iter().any(|byte| matches!(byte, b'\n' | b'\r'))
}

fn shell_command_observations(
    input: &[u8],
    output: &[u8],
    records: Vec<ShellCommandRecord>,
) -> ShellCommandObservations {
    ShellCommandObservations::new(records, shell_command_fallback_observation(input, output))
}

fn shell_command_fallback_observation(input: &[u8], output: &[u8]) -> ShellCommandObservation {
    let lines = shell_command_output_lines(input, output);
    ShellCommandObservation {
        command: None,
        evidence: None,
        diagnostic: shell_command_diagnostic(&lines),
        exit_code: shell_command_exit_code(&lines),
        terminal_output: shell_command_terminal_output(&lines),
    }
}

fn shell_command_output_lines(input: &[u8], output: &[u8]) -> Vec<String> {
    let echoed_lines = String::from_utf8_lossy(input)
        .replace('\r', "\n")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let output = String::from_utf8_lossy(output).replace('\r', "\n");
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "$" && !echoed_lines.iter().any(|echoed| echoed == line))
        .map(str::to_owned)
        .collect()
}

fn shell_command_exit_code(lines: &[String]) -> Option<i32> {
    lines.iter().rev().find_map(|line| {
        line.strip_prefix("qjs exit ")
            .and_then(|value| value.trim().parse::<i32>().ok())
    })
}

fn shell_command_diagnostic(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|line| {
        is_shell_diagnostic_line(line)
            .then(|| truncate_shell_text(line, SHELL_OPERATION_DIAGNOSTIC_MAX_CHARS))
    })
}

fn is_shell_diagnostic_line(line: &str) -> bool {
    line.contains(": ")
        && [
            "errno",
            "not found",
            "missing",
            "usage",
            "invalid",
            "failed",
            "directory not empty",
            "not a directory",
            "service paths",
        ]
        .iter()
        .any(|needle| line.contains(needle))
}

fn shell_command_terminal_output(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    Some(truncate_shell_text(
        &lines.join("\n"),
        SHELL_OPERATION_OUTPUT_MAX_CHARS,
    ))
}

fn truncate_shell_text(value: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}
