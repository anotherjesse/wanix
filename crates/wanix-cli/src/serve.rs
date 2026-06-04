use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use tungstenite::{Error as WsError, Message, WebSocket, accept};
use wanix_fs::{FileSystem, LocalFs, NormalizedPath};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

use crate::json::json_string;
use crate::p9_ws::{P9WsConnectionError, serve_websocket_connection};
use crate::qemu::DEFAULT_P9_MSIZE;
use crate::qjs_term::QjsShellSession;
use crate::rootfs::rootfs_json_handoff_for_prepared_root;
use crate::{CliError, quickjs_runner, write_process_output};

mod boot;
mod html;
mod http;
mod terminal_ws;

use boot::{first_executable_init_route, first_existing_static_route};
use html::{direct_v86_bundle_html, fs9p_bundle_html, workbench_fs9p_bundle_html};
use http::{
    HttpStatus, StaticResponse, header_end, parse_http_request, percent_decode,
    read_static_response,
};
use terminal_ws::close_terminal_websocket_if_finished;

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7654";
const DIRECT_V86_BUNDLE: &str = "direct-v86";
const FS9P_BUNDLE: &str = "fs9p";
const WORKBENCH_FS9P_BUNDLE: &str = "workbench-fs9p";
const QJS_SHELL_WEBSOCKET_PATH: &str = "/.well-known/qjs-shell";
const QJS_SHELL_WEBSOCKET_IDLE_PUMP_MS: u64 = 20;
const DIRECT_V86_DEFAULT_CMDLINE: &str = "console=hvc0 init=/bin/init rw root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L,aname=,cache=none,msize=131072 loglevel=3";
const DIRECT_V86_DEFAULT_P9_MSIZE: u32 = DEFAULT_P9_MSIZE;
const DIRECT_V86_DEFAULT_KERNEL_PATH: &str = "/boot/bzImage";
const DIRECT_V86_INIT_PATH: &str = "/bin/init";
const DIRECT_V86_MEMORY_SIZE: u32 = 1024 * 1024 * 1024;
const DIRECT_V86_VGA_MEMORY_SIZE: u32 = 8 * 1024 * 1024;
const DIRECT_V86_MODULE_PATH: &str = "/v86/lib/libv86.mjs";
const DIRECT_V86_MOD_REEXPORT_PATH: &str = "/v86/lib/mod.js";
const DIRECT_V86_OFFSCREEN_PATH: &str = "/v86/lib/offscreen.js";
const DIRECT_V86_WASM_PATH: &str = "/v86/bundle/v86.wasm";
const DIRECT_V86_BIOS_PATH: &str = "/v86/bundle/seabios.bin";
const DIRECT_V86_VGA_BIOS_PATH: &str = "/v86/bundle/vgabios.bin";
const DIRECT_V86_KERNEL_CANDIDATES: &[&str] = &[DIRECT_V86_DEFAULT_KERNEL_PATH, "/bzImage"];
const DIRECT_V86_INITRD_CANDIDATES: &[&str] =
    &["/boot/initrd", "/boot/initrd.img", "/initrd", "/initrd.img"];

struct BuiltinAsset {
    route: &'static str,
    content_type: &'static str,
    bytes: &'static [u8],
}

const DIRECT_V86_ASSETS: &[BuiltinAsset] = &[
    BuiltinAsset {
        route: DIRECT_V86_MOD_REEXPORT_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../v86/lib/mod.js"),
    },
    BuiltinAsset {
        route: DIRECT_V86_MODULE_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../v86/lib/libv86.mjs"),
    },
    BuiltinAsset {
        route: DIRECT_V86_OFFSCREEN_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../v86/lib/offscreen.js"),
    },
    BuiltinAsset {
        route: DIRECT_V86_WASM_PATH,
        content_type: "application/wasm",
        bytes: include_bytes!("../../../v86/bundle/v86.wasm"),
    },
    BuiltinAsset {
        route: DIRECT_V86_BIOS_PATH,
        content_type: "application/octet-stream",
        bytes: include_bytes!("../../../v86/bundle/seabios.bin"),
    },
    BuiltinAsset {
        route: DIRECT_V86_VGA_BIOS_PATH,
        content_type: "application/octet-stream",
        bytes: include_bytes!("../../../v86/bundle/vgabios.bin"),
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServeCommand {
    root_path: PathBuf,
    addr: String,
    bundle: Option<String>,
    wanix_services: bool,
    once: bool,
}

pub(super) fn parse_serve_command(args: &[OsString]) -> Result<ServeCommand, CliError> {
    let mut root_path = None;
    let mut addr = None;
    let mut bundle = None;
    let mut wanix_services = false;
    let mut once = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("serve --root expects DIR"))?;
            if root_path.is_some() {
                return Err(CliError::usage("serve accepts only one --root"));
            }
            root_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--addr" || args[i] == "--listen" {
            let option = args[i].to_string_lossy();
            i += 1;
            let raw_value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("serve {option} expects HOST:PORT")))?;
            if addr.is_some() {
                return Err(CliError::usage("serve accepts only one --addr or --listen"));
            }
            let value = raw_value.to_string_lossy();
            addr = Some(normalize_listen_addr(&value));
            i += 1;
        } else if args[i] == "--bundle" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("serve --bundle expects NAME"))?;
            if bundle.is_some() {
                return Err(CliError::usage("serve accepts only one --bundle"));
            }
            bundle = Some(value.to_string_lossy().into_owned());
            i += 1;
        } else if args[i] == "--once" {
            if once {
                return Err(CliError::usage("serve accepts only one --once"));
            }
            once = true;
            i += 1;
        } else if args[i] == "--wanix-services" {
            if wanix_services {
                return Err(CliError::usage("serve accepts only one --wanix-services"));
            }
            wanix_services = true;
            i += 1;
        } else if args[i].to_string_lossy().starts_with('-') {
            return Err(CliError::usage(format!(
                "unexpected serve argument: {}",
                args[i].to_string_lossy()
            )));
        } else if root_path.is_some() {
            return Err(CliError::usage("serve accepts only one directory"));
        } else {
            root_path = Some(PathBuf::from(&args[i]));
            i += 1;
        }
    }

    let root_path = root_path.unwrap_or_else(|| PathBuf::from("."));
    let addr = addr.unwrap_or_else(|| DEFAULT_SERVE_ADDR.to_owned());
    Ok(ServeCommand {
        root_path,
        addr,
        bundle,
        wanix_services,
        once,
    })
}

fn normalize_listen_addr(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix(':') {
        format!("0.0.0.0:{port}")
    } else {
        addr.to_owned()
    }
}

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

fn serve_terminal_websocket_connection(
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
        match message {
            Message::Binary(bytes) => {
                let output = session
                    .input(bytes.as_ref())
                    .map_err(ServeConnectionError::Terminal)?;
                send_terminal_output(&mut socket, output)?;
                if close_terminal_websocket_if_finished(&mut socket, &mut session)? {
                    return Ok(());
                }
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
                send_terminal_output(&mut socket, output)?;
                if close_terminal_websocket_if_finished(&mut socket, &mut session)? {
                    return Ok(());
                }
            }
            Message::Close(_) => {
                session
                    .close_terminal_resource()
                    .map_err(ServeConnectionError::Terminal)?;
                return Ok(());
            }
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).map_err(|error| {
                ServeConnectionError::WebSocket(P9WsConnectionError::WebSocket(error))
            })?,
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn is_terminal_websocket_idle_tick(error: &WsError) -> bool {
    matches!(
        error,
        WsError::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            )
    )
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

fn parse_terminal_resize_message(text: &str) -> Option<(u16, u16)> {
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

fn peek_request_target(header_bytes: &[u8]) -> Option<&str> {
    let header_end = header_end(header_bytes)?;
    let header = std::str::from_utf8(&header_bytes[..header_end]).ok()?;
    let request_line = header.lines().next()?;
    let mut parts = request_line.split_whitespace();
    parts.next()?;
    parts.next()
}

fn peek_request_headers(stream: &TcpStream) -> io::Result<Vec<u8>> {
    let mut buffer = [0; MAX_HTTP_HEADER_BYTES];
    loop {
        let len = stream.peek(&mut buffer)?;
        if len == 0 || header_end(&buffer[..len]).is_some() || len == buffer.len() {
            return Ok(buffer[..len].to_vec());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn is_websocket_upgrade(header_bytes: &[u8]) -> bool {
    let header = String::from_utf8_lossy(header_bytes);
    let mut has_connection_upgrade = false;
    let mut has_websocket_upgrade = false;
    for line in header.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("connection") {
            has_connection_upgrade = value
                .split([' ', ','])
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"));
        } else if name.trim().eq_ignore_ascii_case("upgrade") {
            has_websocket_upgrade = value.trim().eq_ignore_ascii_case("websocket");
        }
    }
    has_connection_upgrade && has_websocket_upgrade
}

fn serve_http_connection(
    roots: &ServeRoots,
    mut stream: TcpStream,
    peer_addr: SocketAddr,
) -> Result<(), ServeConnectionError> {
    let request = read_http_request(&mut stream)?;
    let response = match parse_http_request(&request) {
        Ok(path) => well_known_response(roots, &path, &request, peer_addr)
            .or_else(|| bundle_response(roots, &path, &request))
            .or_else(|| direct_v86_asset_response(roots, &path))
            .unwrap_or_else(|| read_static_response(&roots.static_root, &path)),
        Err(status) => StaticResponse::plain(status, status.reason()),
    };
    write_static_response(stream, response)
}

fn write_static_response(
    mut stream: TcpStream,
    response: StaticResponse,
) -> Result<(), ServeConnectionError> {
    stream.write_all(&response.encode())?;
    stream.flush()?;
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, ServeConnectionError> {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while request.len() < MAX_HTTP_HEADER_BYTES {
        let len = stream.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..len]);
        if header_end(&request).is_some() {
            return Ok(request);
        }
    }
    Err(ServeConnectionError::Http(
        "HTTP request header was incomplete or too large".to_owned(),
    ))
}

fn websocket_rejection_response(raw_path: Option<&str>) -> Option<StaticResponse> {
    let raw_path = raw_path?;
    let path = raw_path.split_once('?').map_or(raw_path, |(path, _)| path);
    if path == "/.well-known/export9p" {
        return None;
    }
    if path == "/.well-known/ethernet" {
        return Some(StaticResponse::plain(
            HttpStatus::NotImplemented,
            "ethernet websocket bridge is not implemented in rust serve",
        ));
    }
    if path.starts_with("/.well-known/") {
        return Some(StaticResponse::plain(HttpStatus::NotFound, "not found"));
    }
    None
}

fn well_known_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
) -> Option<StaticResponse> {
    let mut components = relative_path.components();
    match components.next() {
        Some(Component::Normal(component)) if component == ".well-known" => {}
        _ => return None,
    }
    let endpoint = components.next();
    let has_extra_components = components.next().is_some();
    match endpoint {
        Some(Component::Normal(component)) if component == "wanix.json" => {
            if has_extra_components {
                Some(StaticResponse::plain(HttpStatus::NotFound, "not found"))
            } else {
                Some(serve_discovery_response(roots, request, peer_addr))
            }
        }
        Some(Component::Normal(component)) if component == "rootfs.json" => {
            if has_extra_components {
                Some(StaticResponse::plain(HttpStatus::NotFound, "not found"))
            } else {
                Some(rootfs_handoff_response(roots, peer_addr))
            }
        }
        Some(Component::Normal(component)) if component == "export9p" && !has_extra_components => {
            Some(StaticResponse::plain(
                HttpStatus::BadRequest,
                "websocket upgrade required",
            ))
        }
        Some(Component::Normal(component))
            if component == "qjs-shell" && !has_extra_components && roots.wanix_services =>
        {
            Some(StaticResponse::plain(
                HttpStatus::BadRequest,
                "websocket upgrade required",
            ))
        }
        Some(Component::Normal(component)) if component == "ethernet" && !has_extra_components => {
            Some(StaticResponse::plain(
                HttpStatus::NotImplemented,
                "ethernet websocket bridge is not implemented in rust serve",
            ))
        }
        _ => Some(StaticResponse::plain(HttpStatus::NotFound, "not found")),
    }
}

fn bundle_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
) -> Option<StaticResponse> {
    if relative_path != Path::new("index.html") {
        return None;
    }
    let target = http_request_target(request)?;
    let (_, query) = target.split_once('?')?;
    match (roots.bundle.as_deref(), query_param(query, "bundle")) {
        (Some(DIRECT_V86_BUNDLE), Some(DIRECT_V86_BUNDLE)) => Some(direct_v86_bundle_response()),
        (Some(FS9P_BUNDLE), Some(FS9P_BUNDLE)) => Some(fs9p_bundle_response()),
        (Some(WORKBENCH_FS9P_BUNDLE), Some(WORKBENCH_FS9P_BUNDLE)) => {
            Some(workbench_fs9p_bundle_response())
        }
        _ => None,
    }
}

fn direct_v86_asset_response(roots: &ServeRoots, relative_path: &Path) -> Option<StaticResponse> {
    if roots.bundle.as_deref() != Some(DIRECT_V86_BUNDLE) {
        return None;
    }
    let route = format!("/{}", relative_path.to_str()?);
    let asset = DIRECT_V86_ASSETS
        .iter()
        .find(|asset| asset.route == route)?;
    Some(StaticResponse {
        status: HttpStatus::Ok,
        content_type: asset.content_type,
        body: asset.bytes.to_vec(),
    })
}

fn http_request_target(header_bytes: &[u8]) -> Option<&str> {
    let header_end = header_end(header_bytes)?;
    let header = std::str::from_utf8(&header_bytes[..header_end]).ok()?;
    let request_line = header.lines().next()?;
    let mut parts = request_line.split_whitespace();
    parts.next()?;
    parts.next()
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Some(value);
        }
    }
    None
}

fn direct_v86_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: direct_v86_bundle_html().into_bytes(),
    }
}

fn fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: fs9p_bundle_html().into_bytes(),
    }
}

fn workbench_fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: workbench_fs9p_bundle_html().into_bytes(),
    }
}

fn serve_discovery_response(
    roots: &ServeRoots,
    request: &[u8],
    peer_addr: SocketAddr,
) -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "application/json",
        body: serve_discovery_json(roots, request, peer_addr).into_bytes(),
    }
}

fn serve_discovery_json(roots: &ServeRoots, request: &[u8], peer_addr: SocketAddr) -> String {
    let host = request_host(request).unwrap_or_else(|| display_host(roots.local_addr));
    let p9_url = format!("ws://{host}/.well-known/export9p");
    let rootfs_url = format!("http://{host}/.well-known/rootfs.json");
    let qjs_shell_url = format!("ws://{host}{QJS_SHELL_WEBSOCKET_PATH}");
    let ethernet_url = format!("ws://{host}/.well-known/ethernet");
    let bundle = roots
        .bundle
        .as_deref()
        .map(json_string)
        .unwrap_or_else(|| "null".to_owned());
    let services = serve_services_json(roots);
    let qjs_shell_route = serve_qjs_shell_route_json(roots, &qjs_shell_url);
    let rootfs_route = serve_rootfs_route_json(&roots.static_root, &rootfs_url, peer_addr);
    let direct_v86_boot = direct_v86_boot_json(&roots.static_root);
    format!(
        "{{\"version\":1,\
         \"runtime\":\"wanix-rust\",\
         \"routes\":{{\
         \"p9\":{{\"websocket\":{},\"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\",\
         \"supportedProtocols\":[\"9P2000.L\",\"9P2000.L.Google.2\"]}},\
         \"rootfs\":{},\
         \"qjsShell\":{},\
         \"ethernet\":{{\"websocket\":{},\"status\":\"not-implemented\"}}\
         }},\
         \"v86\":{{\"assets\":{{\"module\":{},\"mod\":{},\"offscreen\":{},\"wasm\":{},\"bios\":{},\"vgaBios\":{}}},\
         \"boot\":{},\
         \"defaultCmdline\":{},\"p9Msize\":{},\"memorySize\":{},\"vgaMemorySize\":{},\"virtioConsole\":true}},\
         \"services\":{},\
         \"bundle\":{}}}",
        json_string(&p9_url),
        rootfs_route,
        qjs_shell_route,
        json_string(&ethernet_url),
        json_string(DIRECT_V86_MODULE_PATH),
        json_string(DIRECT_V86_MOD_REEXPORT_PATH),
        json_string(DIRECT_V86_OFFSCREEN_PATH),
        json_string(DIRECT_V86_WASM_PATH),
        json_string(DIRECT_V86_BIOS_PATH),
        json_string(DIRECT_V86_VGA_BIOS_PATH),
        direct_v86_boot,
        json_string(DIRECT_V86_DEFAULT_CMDLINE),
        DIRECT_V86_DEFAULT_P9_MSIZE,
        DIRECT_V86_MEMORY_SIZE,
        DIRECT_V86_VGA_MEMORY_SIZE,
        services,
        bundle
    )
}

fn rootfs_handoff_response(roots: &ServeRoots, peer_addr: SocketAddr) -> StaticResponse {
    if !is_loopback_peer(peer_addr) {
        return StaticResponse::plain(
            HttpStatus::Forbidden,
            "rootfs handoff is available only to loopback clients",
        );
    }
    match rootfs_json_handoff_for_prepared_root(&roots.static_root) {
        Ok(body) => StaticResponse {
            status: HttpStatus::Ok,
            content_type: "application/json",
            body: body.into_bytes(),
        },
        Err(error) if error.exit_code() == 2 => {
            StaticResponse::plain(HttpStatus::BadRequest, &error.to_string())
        }
        Err(error) => StaticResponse::plain(HttpStatus::Conflict, &error.to_string()),
    }
}

fn serve_rootfs_route_json(static_root: &Path, url: &str, peer_addr: SocketAddr) -> String {
    let readiness = rootfs_handoff_readiness(static_root);
    let loopback = is_loopback_peer(peer_addr);
    let mut ready = readiness.missing.is_empty();
    let mut error = None;
    let status = if !loopback {
        "local-only"
    } else if readiness.missing.is_empty() {
        match rootfs_json_handoff_for_prepared_root(static_root) {
            Ok(_) => "available",
            Err(manifest_error) => {
                ready = false;
                error = Some(manifest_error.to_string());
                "invalid"
            }
        }
    } else {
        "unprepared"
    };
    let mut fields = vec![
        format!("\"url\":{}", json_string(url)),
        "\"kind\":\"wanix-rootfs.v1\"".to_owned(),
        format!("\"status\":{}", json_string(status)),
        format!("\"ready\":{}", ready),
    ];
    if !readiness.missing.is_empty() {
        let missing = readiness
            .missing
            .iter()
            .map(|route| json_string(route))
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"missing\":[{missing}]"));
    }
    if let Some(error) = error {
        fields.push(format!("\"error\":{}", json_string(&error)));
    }
    format!("{{{}}}", fields.join(","))
}

struct RootfsHandoffReadiness {
    missing: Vec<&'static str>,
}

fn rootfs_handoff_readiness(static_root: &Path) -> RootfsHandoffReadiness {
    let kernel = first_existing_static_route(static_root, DIRECT_V86_KERNEL_CANDIDATES);
    let init = first_executable_init_route(static_root, DIRECT_V86_INIT_PATH);
    let mut missing = Vec::new();
    if kernel.is_none() {
        missing.push(DIRECT_V86_DEFAULT_KERNEL_PATH);
    }
    if init.is_none() {
        missing.push(DIRECT_V86_INIT_PATH);
    }
    RootfsHandoffReadiness { missing }
}

fn is_loopback_peer(peer_addr: SocketAddr) -> bool {
    peer_addr.ip().is_loopback()
}

fn serve_services_json(roots: &ServeRoots) -> String {
    if roots.wanix_services {
        "{\"task\":\"#task\",\"term\":\"#term\",\"drivers\":[\"noop\",\"qjs\"]}".to_owned()
    } else {
        "null".to_owned()
    }
}

fn serve_qjs_shell_route_json(roots: &ServeRoots, websocket_url: &str) -> String {
    if roots.wanix_services {
        format!(
            "{{\"websocket\":{},\"protocol\":\"wanix-qjs-shell.v1\",\
             \"mode\":\"raw-bytes\",\"status\":\"available\",\
             \"cwdQuery\":\"cwd\",\"defaultCwd\":\".\",\
             \"resize\":[\"{{\\\"type\\\":\\\"resize\\\",\\\"columns\\\":COLS,\\\"rows\\\":ROWS}}\",\
             \"resize COLS ROWS\"],\
             \"exitMessage\":\"{{\\\"type\\\":\\\"exit\\\",\\\"code\\\":N}}\",\
             \"terminalLifecycle\":\"owned-resource-closed-on-session-close\"}}",
            json_string(websocket_url)
        )
    } else {
        "{\"status\":\"disabled\"}".to_owned()
    }
}

fn direct_v86_boot_json(static_root: &Path) -> String {
    let readiness = rootfs_handoff_readiness(static_root);
    let mut fields = Vec::new();
    let kernel = first_existing_static_route(static_root, DIRECT_V86_KERNEL_CANDIDATES);
    let initrd = first_existing_static_route(static_root, DIRECT_V86_INITRD_CANDIDATES);
    let init = first_executable_init_route(static_root, DIRECT_V86_INIT_PATH);
    if let Some(kernel) = kernel {
        fields.push(format!("\"kernel\":{}", json_string(kernel)));
    }
    if let Some(initrd) = initrd {
        fields.push(format!("\"initrd\":{}", json_string(initrd)));
    }
    if let Some(init) = init {
        fields.push(format!("\"init\":{}", json_string(init)));
    }
    fields.push(format!("\"ready\":{}", readiness.missing.is_empty()));
    if !readiness.missing.is_empty() {
        let missing = readiness
            .missing
            .into_iter()
            .map(json_string)
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"missing\":[{missing}]"));
    }
    format!("{{{}}}", fields.join(","))
}

fn request_host(request: &[u8]) -> Option<String> {
    let header_end = header_end(request)?;
    let header = std::str::from_utf8(&request[..header_end]).ok()?;
    for line in header.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("host") {
            let host = value.trim();
            if is_safe_host(host) {
                return Some(host.to_owned());
            }
        }
    }
    None
}

fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && host.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'-'
                    | b'_'
                    | b':'
                    | b'['
                    | b']'
            )
        })
}

fn display_host(local_addr: SocketAddr) -> String {
    let host = if local_addr.ip().is_unspecified() {
        "localhost".to_owned()
    } else if local_addr.ip().is_ipv6() {
        format!("[{}]", local_addr.ip())
    } else {
        local_addr.ip().to_string()
    };
    format!("{host}:{}", local_addr.port())
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
