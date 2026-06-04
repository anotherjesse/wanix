use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use tungstenite::accept;
use wanix_fs::{FileSystem, LocalFs, NormalizedPath};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

use crate::p9_ws::{P9WsConnectionError, serve_websocket_connection};
use crate::{CliError, quickjs_runner, write_process_output};

mod boot;
mod command;
mod direct_v86;
mod discovery;
mod html;
mod http;
mod terminal_ws;

#[cfg(test)]
pub(super) use command::DEFAULT_SERVE_ADDR;
pub(super) use command::{ServeCommand, parse_serve_command};
use discovery::display_host;
use http::{
    HttpStatus, StaticResponse, is_websocket_upgrade, peek_request_headers, peek_request_target,
    percent_decode, serve_http_connection, websocket_rejection_response, write_static_response,
};
use terminal_ws::serve_terminal_websocket_connection;

const FS9P_BUNDLE: &str = "fs9p";
const WORKBENCH_FS9P_BUNDLE: &str = "workbench-fs9p";
const QJS_SHELL_WEBSOCKET_PATH: &str = "/.well-known/qjs-shell";
const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;

pub(super) fn run_serve_streaming(
    command: ServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let listener = TcpListener::bind(&command.addr).map_err(|error| {
        CliError::new(
            format!("failed to bind serve address {}: {error}", command.addr),
            1,
        )
    })?;
    run_serve_with_listener(command, listener, process_stderr)
}

fn run_serve_with_listener(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_serve_with_listener_inner(command, listener, process_stderr, None)
}

fn run_serve_with_listener_inner(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
    concurrent_connection_limit: Option<usize>,
) -> Result<i32, CliError> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| CliError::new(format!("failed to inspect serve address: {error}"), 1))?;
    let roots = ServeRoots::new(
        &command.root_path,
        local_addr,
        command.bundle.clone(),
        command.wanix_services,
    )?;

    write_process_output(
        process_stderr,
        "stderr",
        format!(
            "wanix-rust serve: serving {} files with Wanix overlay\n",
            roots.static_root.display()
        )
        .as_bytes(),
    )?;
    write_process_output(
        process_stderr,
        "stderr",
        serve_url_status(local_addr, command.bundle.as_deref()).as_bytes(),
    )?;

    if command.once {
        return serve_one_connection(&listener, &roots, process_stderr);
    }

    serve_concurrent_connections(listener, roots, process_stderr, concurrent_connection_limit)
}

#[cfg(test)]
fn run_serve_with_listener_for_connections(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
    connection_limit: usize,
) -> Result<i32, CliError> {
    run_serve_with_listener_inner(command, listener, process_stderr, Some(connection_limit))
}

fn serve_concurrent_connections(
    listener: TcpListener,
    roots: ServeRoots,
    process_stderr: &mut dyn Write,
    connection_limit: Option<usize>,
) -> Result<i32, CliError> {
    listener.set_nonblocking(true).map_err(|error| {
        CliError::new(
            format!("failed to configure serve listener as nonblocking: {error}"),
            1,
        )
    })?;
    let roots = Arc::new(roots);
    let (error_sender, error_receiver) = mpsc::channel::<String>();
    let mut accepted = 0usize;
    let mut handles = Vec::new();
    let mut had_error = false;

    loop {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    had_error = true;
                    write_process_output(
                        process_stderr,
                        "stderr",
                        format!(
                            "wanix-rust serve: connection {peer_addr} failed: \
                             could not configure blocking mode: {error}\n"
                        )
                        .as_bytes(),
                    )?;
                    continue;
                }
                accepted += 1;
                let connection_roots = Arc::clone(&roots);
                let connection_errors = error_sender.clone();
                let handle = thread::spawn(move || {
                    if let Err(error) = serve_connection(&connection_roots, stream, peer_addr) {
                        let _ = connection_errors.send(format!(
                            "wanix-rust serve: connection {peer_addr} failed: {error}\n"
                        ));
                    }
                });
                if connection_limit.is_some() {
                    handles.push(handle);
                }
                if connection_limit.is_some_and(|limit| accepted >= limit) {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(CliError::new(format!("serve accept failed: {error}"), 1)),
        }
        had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
    }

    drop(error_sender);
    for handle in handles {
        if handle.join().is_err() {
            had_error = true;
            write_process_output(
                process_stderr,
                "stderr",
                b"wanix-rust serve: connection worker panicked\n",
            )?;
        }
    }
    had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
    Ok(i32::from(had_error))
}

fn drain_connection_errors(
    error_receiver: &mpsc::Receiver<String>,
    process_stderr: &mut dyn Write,
) -> Result<bool, CliError> {
    let mut had_error = false;
    while let Ok(message) = error_receiver.try_recv() {
        had_error = true;
        write_process_output(process_stderr, "stderr", message.as_bytes())?;
        write_process_output(
            process_stderr,
            "stderr",
            b"wanix-rust serve: continuing after connection error\n",
        )?;
    }
    Ok(had_error)
}

fn serve_url_status(local_addr: SocketAddr, bundle: Option<&str>) -> String {
    let host = display_host(local_addr);
    match bundle {
        Some(bundle) => {
            format!("wanix-rust serve: bundle available at http://{host}/?bundle={bundle}\n")
        }
        None => format!("wanix-rust serve: listening on http://{host}/\n"),
    }
}

#[derive(Clone)]
struct ServeRoots {
    static_root: PathBuf,
    p9_root: Arc<dyn FileSystem>,
    local_addr: SocketAddr,
    bundle: Option<String>,
    wanix_services: bool,
}

impl ServeRoots {
    fn new(
        root_path: &Path,
        local_addr: SocketAddr,
        bundle: Option<String>,
        wanix_services: bool,
    ) -> Result<Self, CliError> {
        let static_root = fs::canonicalize(root_path).map_err(|error| {
            CliError::new(
                format!("failed to open serve root {}: {error}", root_path.display()),
                1,
            )
        })?;
        let p9_root = serve_p9_root(root_path, wanix_services)?;
        Ok(Self {
            static_root,
            p9_root,
            local_addr,
            bundle,
            wanix_services,
        })
    }
}

fn serve_p9_root(root_path: &Path, wanix_services: bool) -> Result<Arc<dyn FileSystem>, CliError> {
    let host_root = Arc::new(LocalFs::new(root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open serve 9P root {}: {error}",
                root_path.display()
            ),
            1,
        )
    })?);
    if !wanix_services {
        return Ok(host_root);
    }

    let table = serve_task_table()?;
    let terminal = Arc::new(TermDevice::new());
    let mut namespace = Namespace::new();
    namespace.bind(host_root, ".", ".", BindOptions::default())?;
    namespace.bind(terminal, ".", "#term", BindOptions::default())?;
    let root_task = table.allocate_root_with_namespace("noop", namespace.clone())?;
    namespace.bind(
        Arc::new(table.filesystem_for(root_task.id())),
        ".",
        "#task",
        BindOptions {
            position: BindPosition::Replace,
        },
    )?;
    Ok(Arc::new(namespace))
}

fn serve_task_table() -> Result<TaskTable, CliError> {
    let table = TaskTable::new();
    table.register_noop_driver("noop")?;
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
    Ok(table)
}

fn serve_one_connection(
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

fn serve_connection(
    roots: &ServeRoots,
    stream: TcpStream,
    peer_addr: SocketAddr,
) -> Result<(), ServeConnectionError> {
    let request = peek_request_headers(&stream)?;
    if is_websocket_upgrade(&request) {
        let target = peek_request_target(&request);
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
            let socket = accept(stream).map_err(|error| {
                ServeConnectionError::WebSocket(P9WsConnectionError::Handshake(error.to_string()))
            })?;
            return serve_terminal_websocket_connection(&roots.static_root, &cwd, socket);
        }
        if let Some(response) = websocket_rejection_response(target) {
            return write_static_response(stream, response);
        }
        let socket = accept(stream).map_err(|error| {
            ServeConnectionError::WebSocket(P9WsConnectionError::Handshake(error.to_string()))
        })?;
        return serve_websocket_connection(Arc::clone(&roots.p9_root), socket)
            .map_err(ServeConnectionError::WebSocket);
    }

    serve_http_connection(roots, stream, peer_addr)
}

fn is_qjs_shell_websocket_path(raw_path: Option<&str>) -> bool {
    raw_path.map(|path| path.split_once('?').map_or(path, |(path, _)| path))
        == Some(QJS_SHELL_WEBSOCKET_PATH)
}

fn qjs_shell_cwd_from_target(raw_path: Option<&str>) -> Result<NormalizedPath, StaticResponse> {
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

#[derive(Debug)]
enum ServeConnectionError {
    Io(io::Error),
    Http(String),
    WebSocket(P9WsConnectionError),
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

impl From<io::Error> for ServeConnectionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests;
