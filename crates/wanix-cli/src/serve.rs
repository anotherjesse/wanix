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
mod terminal_ws;

use boot::{first_executable_init_route, first_existing_static_route};
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

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_http_request(bytes: &[u8]) -> Result<PathBuf, HttpStatus> {
    let header_end = header_end(bytes).ok_or(HttpStatus::BadRequest)?;
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| HttpStatus::BadRequest)?;
    let mut lines = header.lines();
    let request_line = lines.next().ok_or(HttpStatus::BadRequest)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(HttpStatus::BadRequest)?;
    let raw_path = parts.next().ok_or(HttpStatus::BadRequest)?;
    let version = parts.next().ok_or(HttpStatus::BadRequest)?;
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        return Err(HttpStatus::BadRequest);
    }
    if method != "GET" {
        return Err(HttpStatus::MethodNotAllowed);
    }
    safe_relative_path(raw_path)
}

fn safe_relative_path(raw_path: &str) -> Result<PathBuf, HttpStatus> {
    let path_without_query = raw_path.split_once('?').map_or(raw_path, |(path, _)| path);
    if !path_without_query.starts_with('/') {
        return Err(HttpStatus::BadRequest);
    }

    let mut path = PathBuf::new();
    for raw_segment in path_without_query.split('/') {
        if raw_segment.is_empty() {
            continue;
        }
        let segment = percent_decode(raw_segment)?;
        if segment == "." || segment == ".." || segment.contains('/') || segment.contains('\\') {
            return Err(HttpStatus::Forbidden);
        }
        path.push(segment);
    }

    if path.as_os_str().is_empty() {
        path.push("index.html");
    }
    Ok(path)
}

fn percent_decode(segment: &str) -> Result<String, HttpStatus> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = *bytes.get(i + 1).ok_or(HttpStatus::BadRequest)?;
            let low = *bytes.get(i + 2).ok_or(HttpStatus::BadRequest)?;
            decoded.push((hex_value(high)? << 4) | hex_value(low)?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| HttpStatus::BadRequest)
}

fn hex_value(byte: u8) -> Result<u8, HttpStatus> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(HttpStatus::BadRequest),
    }
}

fn read_static_response(static_root: &Path, relative_path: &Path) -> StaticResponse {
    let candidate = static_root.join(relative_path);
    let candidate = if candidate.is_dir() {
        candidate.join("index.html")
    } else {
        candidate
    };

    let canonical = match fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(_) => return StaticResponse::plain(HttpStatus::NotFound, "not found"),
    };
    if !canonical.starts_with(static_root) {
        return StaticResponse::plain(HttpStatus::Forbidden, "forbidden");
    }
    match fs::read(&canonical) {
        Ok(body) => StaticResponse {
            status: HttpStatus::Ok,
            content_type: content_type(&canonical),
            body,
        },
        Err(_) => StaticResponse::plain(HttpStatus::NotFound, "not found"),
    }
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

fn direct_v86_bundle_html() -> String {
    let mut html = String::new();
    html.push_str(r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Wanix Rust direct v86</title>
  <style>
    body { margin: 0; font: 14px system-ui, sans-serif; color: #f4f4f0; background: #171a1f; }
    main { display: grid; grid-template-columns: minmax(280px, 420px) 1fr; min-height: 100vh; }
    aside { padding: 20px; border-right: 1px solid #363b44; background: #20242b; }
    #screen { min-height: 100vh; background: #080a0d; }
    #screen canvas { display: block; max-width: 100%; }
    #screen div { white-space: pre; font: 14px ui-monospace, SFMono-Regular, Menlo, monospace; }
    #serial { min-height: 180px; resize: vertical; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
    #boot-log, #rootfs { max-height: 180px; overflow: auto; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; background: #11141a; padding: 12px; border: 1px solid #363b44; }
    label { display: block; margin: 12px 0 4px; color: #c9d3e0; }
    input, textarea { box-sizing: border-box; width: 100%; padding: 8px; color: #f4f4f0; background: #11141a; border: 1px solid #4b5563; }
    button { margin-top: 14px; padding: 8px 12px; color: #11141a; background: #d8e8a2; border: 0; cursor: pointer; }
    @media (max-width: 760px) { main { grid-template-columns: 1fr; } #screen { min-height: 55vh; } }
  </style>
</head>
<body>
  <main>
    <aside>
      <h1>Wanix Rust direct v86</h1>
      <p id="status">Loading Wanix discovery...</p>
      <label for="kernel">bzImage URL</label>
      <input id="kernel" value="">
      <label for="initrd">initrd URL</label>
      <input id="initrd" value="">
      <label for="cmdline">kernel command line</label>
      <textarea id="cmdline" spellcheck="false"></textarea>
      <button id="start" disabled>Start VM</button>
      <pre id="config"></pre>
      <label for="rootfs">rootfs handoff</label>
      <button id="copy-qemu" disabled>Copy QEMU command</button>
      <button id="copy-serve" disabled>Copy serve command</button>
      <pre id="rootfs" aria-live="polite"></pre>
      <pre id="boot-log" aria-live="polite"></pre>
    </aside>
    <div id="screen"><canvas></canvas><div></div></div>
    <textarea id="serial" spellcheck="false" readonly></textarea>
  </main>
  <script type="module">
"##);
    html.push_str("    const DEFAULT_CMDLINE = ");
    html.push_str(&json_string(DIRECT_V86_DEFAULT_CMDLINE));
    html.push_str(";\n");
    html.push_str("    const DEFAULT_KERNEL_URL = ");
    html.push_str(&json_string(DIRECT_V86_DEFAULT_KERNEL_PATH));
    html.push_str(";\n");
    html.push_str("    const DEFAULT_MEMORY_SIZE = ");
    html.push_str(&DIRECT_V86_MEMORY_SIZE.to_string());
    html.push_str(";\n");
    html.push_str("    const DEFAULT_VGA_MEMORY_SIZE = ");
    html.push_str(&DIRECT_V86_VGA_MEMORY_SIZE.to_string());
    html.push_str(";\n");
    html.push_str(r##"
    const status = document.querySelector("#status");
    const start = document.querySelector("#start");
    const configOutput = document.querySelector("#config");
    const rootfsOutput = document.querySelector("#rootfs");
    const copyQemu = document.querySelector("#copy-qemu");
    const copyServe = document.querySelector("#copy-serve");
    const bootLog = document.querySelector("#boot-log");
    const kernel = document.querySelector("#kernel");
    const initrd = document.querySelector("#initrd");
    const cmdline = document.querySelector("#cmdline");
    const hvc0 = document.querySelector("#serial");
    const params = new URLSearchParams(location.search);
    const hvc0Encoder = new TextEncoder();
    const hvc0Decoder = new TextDecoder();
    window.wanixV86BootLog = [];
    let rootfsHandoffCommands = null;

    function logBoot(message) {
      window.wanixV86BootLog.push(message);
      bootLog.textContent = window.wanixV86BootLog.join("\n") + "\n";
      bootLog.scrollTop = bootLog.scrollHeight;
    }

    function setStatus(message) {
      status.textContent = message;
      window.wanixV86Status = message;
      logBoot(message);
    }
    window.addEventListener("error", event => setStatus("direct-v86 setup failed: " + event.message));
    window.addEventListener("unhandledrejection", event => setStatus("direct-v86 setup failed: " + (event.reason?.message || event.reason)));

    const discovery = await fetch("/.well-known/wanix.json", { cache: "no-store" }).then(response => response.json());
    const v86Assets = discovery.v86?.assets || {};
    const v86Boot = discovery.v86?.boot || {};
    void loadRootfsHandoff(discovery.routes?.rootfs || {});
    const { V86 } = await import(v86Assets.module || "/v86/lib/libv86.mjs");
    const proxyUrl = discovery.routes.p9.websocket;
    kernel.value = v86Boot.kernel || DEFAULT_KERNEL_URL;
    if (v86Boot.initrd) initrd.value = v86Boot.initrd;
    if (params.get("kernel")) kernel.value = params.get("kernel");
    if (params.get("bzimage")) kernel.value = params.get("bzimage");
    if (params.get("initrd")) initrd.value = params.get("initrd");
    cmdline.value = discovery.v86?.defaultCmdline || DEFAULT_CMDLINE;
    if (params.get("cmdline")) cmdline.value = params.get("cmdline");
    if (params.get("p9-msize")) cmdline.value = cmdline.value.replace(/(\bmsize=)\d+/, "$1" + params.get("p9-msize"));
    if (params.get("append")) cmdline.value = [cmdline.value, params.get("append")].filter(Boolean).join(" ");
    logBoot("9P proxy: " + proxyUrl);
    if (v86Boot.ready) {
      logBoot("Boot root ready: " + v86Boot.kernel + " with " + v86Boot.init);
    } else if (Array.isArray(v86Boot.missing) && v86Boot.missing.length) {
      logBoot("Boot root missing: " + v86Boot.missing.join(", "));
    } else {
      logBoot("Boot root readiness unknown");
    }
    setStatus("Ready: 9P proxy " + proxyUrl);

    async function loadRootfsHandoff(route) {
      route = route || {};
      window.wanixRootfsRoute = route;
      window.wanixRootfsHandoff = null;
      rootfsHandoffCommands = null;
      copyQemu.disabled = true;
      copyServe.disabled = true;
      const status = route.status || "unknown";
      const summary = {
        status,
        ready: route.ready === true,
        url: route.url || null,
        missing: Array.isArray(route.missing) ? route.missing : []
      };
      if (status !== "available" || route.ready !== true || !route.url) {
        rootfsOutput.textContent = JSON.stringify(summary, null, 2);
        const missing = summary.missing.length ? ": " + summary.missing.join(", ") : "";
        logBoot("Rootfs handoff " + status + missing);
        return null;
      }
      try {
        const response = await fetch(route.url, { cache: "no-store" });
        if (!response.ok) {
          rootfsOutput.textContent = JSON.stringify({ ...summary, fetchStatus: response.status }, null, 2);
          logBoot("Rootfs handoff fetch failed: HTTP " + response.status);
          return null;
        }
        const handoff = await response.json();
        const qemuArgv = handoff.qemu?.argv || [];
        const serveArgv = handoff.serveDirectV86?.argv || [];
        rootfsHandoffCommands = {
          qemu: shellCommand(qemuArgv),
          serve: shellCommand(serveArgv)
        };
        window.wanixRootfsHandoff = handoff;
        rootfsOutput.textContent = JSON.stringify({
          kind: handoff.kind,
          rootPath: handoff.rootPath,
          kernelRoute: handoff.kernelRoute,
          initRoute: handoff.initRoute,
          qemuArgv,
          qemuCommand: rootfsHandoffCommands.qemu,
          serveArgv,
          serveCommand: rootfsHandoffCommands.serve
        }, null, 2);
        copyQemu.disabled = rootfsHandoffCommands.qemu.length === 0;
        copyServe.disabled = rootfsHandoffCommands.serve.length === 0;
        logBoot("Rootfs handoff ready: " + handoff.rootPath);
        return handoff;
      } catch (error) {
        rootfsOutput.textContent = JSON.stringify({
          ...summary,
          error: error?.message || String(error)
        }, null, 2);
        logBoot("Rootfs handoff fetch failed: " + (error?.message || error));
        return null;
      }
    }

    function shellCommand(argv) {
      if (!Array.isArray(argv)) return "";
      return argv.map(shellQuoteArg).join(" ");
    }

    function shellQuoteArg(value) {
      const text = String(value);
      if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(text)) return text;
      return "'" + text.replace(/'/g, "'\"'\"'") + "'";
    }

    async function copyCommand(kind) {
      const command = rootfsHandoffCommands?.[kind] || "";
      if (!command) return;
      if (!navigator.clipboard?.writeText) {
        logBoot("Clipboard API unavailable for " + kind + " command");
        return;
      }
      await navigator.clipboard.writeText(command);
      logBoot("Copied " + kind + " command");
    }

    copyQemu.addEventListener("click", () => void copyCommand("qemu"));
    copyServe.addEventListener("click", () => void copyCommand("serve"));

    function buildConfig() {
      const config = {
        memory_size: discovery.v86?.memorySize || DEFAULT_MEMORY_SIZE,
        vga_memory_size: discovery.v86?.vgaMemorySize || DEFAULT_VGA_MEMORY_SIZE,
        cmdline: cmdline.value,
        wasm_path: v86Assets.wasm || "/v86/bundle/v86.wasm",
        bios: { url: v86Assets.bios || "/v86/bundle/seabios.bin" },
        vga_bios: { url: v86Assets.vgaBios || "/v86/bundle/vgabios.bin" },
        bzimage_initrd_from_filesystem: false,
        filesystem: { proxy_url: proxyUrl },
        autostart: true,
        virtio_console: true,
        disable_speaker: true,
        disable_mouse: true,
        disable_keyboard: false,
        screen_container: document.querySelector("#screen"),
        serial_container: document.querySelector("#serial")
      };
      if (kernel.value) config.bzimage = { url: kernel.value };
      if (initrd.value) config.initrd = { url: initrd.value };
      return config;
    }

    function refreshConfig() {
      configOutput.textContent = JSON.stringify(buildConfig(), (key, value) => {
        if (key.endsWith("_container")) return "#" + value.id;
        return value;
      }, 2);
    }

    function appendHvc0(bytes) {
      hvc0.value += hvc0Decoder.decode(bytes, { stream: true });
      hvc0.scrollTop = hvc0.scrollHeight;
    }

    function hvc0KeyBytes(event) {
      const control = event.key.toLowerCase(); if (event.ctrlKey && !event.altKey && !event.metaKey && (control === "c" || control === "d")) return Uint8Array.of(control === "c" ? 3 : 4);
      if (event.metaKey || event.altKey || event.ctrlKey) return null;
      if (event.key === "Enter") return hvc0Encoder.encode("\n");
      if (event.key === "Backspace") return Uint8Array.of(0x7f);
      if (event.key === "Tab") return hvc0Encoder.encode("\t");
      if (event.key.length === 1) return hvc0Encoder.encode(event.key);
      return null;
    }

    function hvc0InputBytes(input) { if (input === undefined || input === null) return new Uint8Array(); if (input instanceof Uint8Array) return input; if (input instanceof ArrayBuffer) return new Uint8Array(input); if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength); if (Array.isArray(input)) return Uint8Array.from(input); return hvc0Encoder.encode(String(input)); }
    function sendHvc0(vm, bytes) { vm.bus.send("virtio-console0-input-bytes", bytes); }
    window.wanixV86SendHvc0 = input => { const vm = window.wanixV86; if (!vm) throw new Error("v86 is not started"); sendHvc0(vm, hvc0InputBytes(input)); };

    function sendHvc0Resize(vm) {
      const columns = Math.max(20, Math.floor(hvc0.clientWidth / 8) || 80);
      const rows = Math.max(5, Math.floor(hvc0.clientHeight / 18) || 24);
      vm.bus.send("virtio-console0-resize", [columns, rows]);
    }

    function connectHvc0Resize(vm) {
      const resize = () => sendHvc0Resize(vm);
      if ("ResizeObserver" in window) {
        const observer = new ResizeObserver(resize);
        observer.observe(hvc0);
        window.wanixV86Hvc0ResizeObserver = observer;
      }
      window.addEventListener("resize", resize);
      vm.add_listener("emulator-ready", resize);
      vm.add_listener("emulator-loaded", resize);
    }

    function connectHvc0(vm) {
      vm.add_listener("virtio-console0-output-bytes", appendHvc0);
      hvc0.addEventListener("keydown", event => {
        const bytes = hvc0KeyBytes(event);
        if (!bytes) return;
        event.preventDefault();
        sendHvc0(vm, bytes);
      });
      hvc0.addEventListener("paste", event => {
        const text = event.clipboardData?.getData("text");
        if (!text) return;
        event.preventDefault();
        sendHvc0(vm, hvc0Encoder.encode(text));
      });
      connectHvc0Resize(vm);
    }

    function describeDownloadProgress(progress) {
      if (!progress || typeof progress !== "object") return "download-progress";
      const name = progress.file_name || progress.name || progress.url || "asset";
      const loaded = progress.loaded ?? progress.done;
      const total = progress.total ?? progress.size;
      if (loaded !== undefined && total !== undefined) return name + " " + loaded + "/" + total;
      if (loaded !== undefined) return name + " " + loaded;
      return name;
    }

    function connectBootStatus(vm) {
      vm.add_listener("download-progress", progress => logBoot("download: " + describeDownloadProgress(progress)));
      vm.add_listener("emulator-loaded", () => logBoot("v86 runtime loaded"));
      vm.add_listener("emulator-ready", () => setStatus("v86 ready"));
      vm.add_listener("emulator-started", () => setStatus("v86 started"));
      vm.add_listener("emulator-stopped", () => setStatus("v86 stopped"));
      vm.add_listener("emulator-error", error => setStatus("v86 error: " + (error?.message || error)));
    }

    function startVm() {
      if (window.wanixV86) return;
      start.disabled = true;
      setStatus("Starting v86");
      const vm = new V86(buildConfig());
      window.wanixV86 = vm;
      connectBootStatus(vm);
      connectHvc0(vm);
      hvc0.focus();
      return vm;
    }

    kernel.addEventListener("input", refreshConfig);
    initrd.addEventListener("input", refreshConfig);
    cmdline.addEventListener("input", refreshConfig);
    refreshConfig();
    start.disabled = false;
    window.wanixV86Start = startVm;
    start.addEventListener("click", startVm);
    if (params.get("autostart") === "1") startVm();
  </script>
</body>
</html>
"##);
    html
}

fn fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: fs9p_bundle_html().into_bytes(),
    }
}

fn fs9p_bundle_html() -> String {
    String::from(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Wanix Rust 9P filesystem</title>
  <style>
    :root { color-scheme: dark; }
    body { margin: 0; font: 14px system-ui, sans-serif; color: #eef2f7; background: #121417; }
    main { display: grid; grid-template-columns: minmax(300px, 360px) 1fr; min-height: 100vh; }
    aside { padding: 18px; border-right: 1px solid #303742; background: #1d232b; }
    section { padding: 18px; display: grid; grid-template-columns: minmax(220px, 320px) 1fr; gap: 16px; align-content: start; }
    h1 { margin: 0 0 12px; font-size: 22px; font-weight: 650; letter-spacing: 0; }
    h2 { margin: 0 0 10px; font-size: 15px; font-weight: 650; letter-spacing: 0; }
    label { display: block; margin: 12px 0 4px; color: #c8d1dc; }
    input, textarea, select { box-sizing: border-box; width: 100%; border: 1px solid #46515f; background: #0e1115; color: #eef2f7; padding: 8px; font: inherit; }
    textarea { min-height: 180px; resize: vertical; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
    button { border: 0; background: #9fdcc4; color: #111417; padding: 8px 11px; font: inherit; cursor: pointer; }
    button:disabled { opacity: .45; cursor: default; }
    .row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
    .stack { display: grid; gap: 12px; align-content: start; }
    #entries { display: grid; gap: 4px; }
    #entries button { text-align: left; background: #242c35; color: #eef2f7; border: 1px solid #3a4551; }
    pre { margin: 0; min-height: 160px; white-space: pre-wrap; overflow-wrap: anywhere; border: 1px solid #303742; background: #0e1115; color: #dbe6f1; padding: 10px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
    @media (max-width: 800px) { main, section { grid-template-columns: 1fr; } }
  </style>
</head>
<body>
  <main>
    <aside>
      <h1>Wanix Rust 9P filesystem</h1>
      <p id="status">Disconnected</p>
      <div class="row">
        <button id="connect">Connect</button>
        <button id="refresh" disabled>Refresh</button>
      </div>
      <label for="cwd">Directory</label>
      <input id="cwd" value="/" spellcheck="false">
      <label for="path">Path</label>
      <input id="path" value="/demo.txt" spellcheck="false">
      <div class="row">
        <button id="read" disabled>Read</button>
        <button id="write" disabled>Write</button>
      </div>
      <label for="rename-to">Rename to</label>
      <input id="rename-to" value="/renamed.txt" spellcheck="false">
      <div class="row">
        <button id="rename" disabled>Rename</button>
        <button id="remove" disabled>Delete</button>
      </div>
    </aside>
    <section>
      <div class="stack">
        <div>
          <h2>Entries</h2>
          <div id="entries"></div>
        </div>
        <div>
          <h2>Log</h2>
          <pre id="log"></pre>
        </div>
      </div>
      <div class="stack">
        <div>
          <h2>File Text</h2>
          <textarea id="editor" spellcheck="false">hello from fs9p
</textarea>
        </div>
        <div>
          <h2>Discovery</h2>
          <pre id="discovery"></pre>
        </div>
      </div>
    </section>
  </main>
  <script type="module">
    const P9_TVERSION = 100;
    const P9_RLERROR = 7;
    const P9_TATTACH = 104;
    const P9_TWALK = 110;
    const P9_TLOPEN = 12;
    const P9_TLCREATE = 14;
    const P9_TSYMLINK = 16;
    const P9_TREADLINK = 22;
    const P9_TREAD = 116;
    const P9_TWRITE = 118;
    const P9_TCLUNK = 120;
    const P9_TREADDIR = 40;
    const P9_TRENAMEAT = 74;
    const P9_TUNLINKAT = 76;
    const P9_NOFID = 0xffffffff;
    const P9_VERSION = "9P2000.L";
    const O_RDONLY = 0;
    const O_RDWR = 0o2;
    const O_TRUNC = 0o1000;
    const MODE_FILE = 0o100664;

    const p9Names = new Map([
      [P9_TVERSION, "Tversion"],
      [P9_TATTACH, "Tattach"],
      [P9_TWALK, "Twalk"],
      [P9_TLOPEN, "Tlopen"],
      [P9_TLCREATE, "Tlcreate"],
      [P9_TSYMLINK, "Tsymlink"],
      [P9_TREADLINK, "Treadlink"],
      [P9_TREAD, "Tread"],
      [P9_TWRITE, "Twrite"],
      [P9_TCLUNK, "Tclunk"],
      [P9_TREADDIR, "Treaddir"],
      [P9_TRENAMEAT, "Trenameat"],
      [P9_TUNLINKAT, "Tunlinkat"]
    ]);

    const utf8 = new TextEncoder();
    const text = new TextDecoder();
    const status = document.querySelector("#status");
    const discoveryOutput = document.querySelector("#discovery");
    const entries = document.querySelector("#entries");
    const logOutput = document.querySelector("#log");
    const cwd = document.querySelector("#cwd");
    const pathInput = document.querySelector("#path");
    const renameTo = document.querySelector("#rename-to");
    const editor = document.querySelector("#editor");
    const buttons = {
      connect: document.querySelector("#connect"),
      refresh: document.querySelector("#refresh"),
      read: document.querySelector("#read"),
      write: document.querySelector("#write"),
      rename: document.querySelector("#rename"),
      remove: document.querySelector("#remove")
    };

    let client = null;

    class Writer {
      constructor() { this.parts = []; }
      u8(value) { this.parts.push(value & 0xff); }
      u16(value) { this.parts.push(value & 0xff, value >>> 8 & 0xff); }
      u32(value) {
        this.parts.push(
          value & 0xff,
          value >>> 8 & 0xff,
          value >>> 16 & 0xff,
          value >>> 24 & 0xff
        );
      }
      u64(value) {
        let remaining = BigInt(value);
        for (let i = 0; i < 8; i += 1) {
          this.parts.push(Number(remaining & 0xffn));
          remaining >>= 8n;
        }
      }
      string(value) {
        const bytes = utf8.encode(value);
        this.u16(bytes.length);
        this.data(bytes);
      }
      data(bytes) {
        for (const byte of bytes) this.parts.push(byte);
      }
      done() { return Uint8Array.from(this.parts); }
    }

    class Reader {
      constructor(bytes) { this.bytes = bytes; this.offset = 0; }
      u8() { return this.bytes[this.offset++]; }
      u16() {
        const value = this.bytes[this.offset] | this.bytes[this.offset + 1] << 8;
        this.offset += 2;
        return value;
      }
      u32() {
        const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 4);
        const value = view.getUint32(0, true);
        this.offset += 4;
        return value;
      }
      u64() {
        const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 8);
        const value = view.getBigUint64(0, true);
        this.offset += 8;
        return value;
      }
      qid() {
        return {
          type: this.u8(),
          version: this.u32(),
          path: this.u64().toString()
        };
      }
      string() {
        const len = this.u16();
        const value = text.decode(this.bytes.slice(this.offset, this.offset + len));
        this.offset += len;
        return value;
      }
      data(count) {
        const data = this.bytes.slice(this.offset, this.offset + count);
        this.offset += count;
        return data;
      }
      remaining() { return this.bytes.length - this.offset; }
    }

    class P9Client {
      constructor(url) {
        this.url = url;
        this.rootFid = 1;
        this.nextFid = 2;
        this.nextTag = 1;
        this.pending = new Map();
      }

      async connect() {
        this.socket = new WebSocket(this.url);
        this.socket.binaryType = "arraybuffer";
        this.socket.addEventListener("message", event => this.handleMessage(event));
        await new Promise((resolve, reject) => {
          this.socket.addEventListener("open", resolve, { once: true });
          this.socket.addEventListener("error", reject, { once: true });
        });
        await this.version();
        await this.attach();
      }

      frame(type, tag, payload) {
        const buffer = new ArrayBuffer(7 + payload.length);
        const view = new DataView(buffer);
        view.setUint32(0, buffer.byteLength, true);
        view.setUint8(4, type);
        view.setUint16(5, tag, true);
        new Uint8Array(buffer, 7).set(payload);
        return buffer;
      }

      rpc(type, payload) {
        const tag = this.nextTag++;
        const name = p9Names.get(type) || String(type);
        appendLog("-> " + name);
        this.socket.send(this.frame(type, tag, payload));
        return new Promise((resolve, reject) => {
          this.pending.set(tag, { name, resolve, reject });
        });
      }

      handleMessage(event) {
        const bytes = new Uint8Array(event.data);
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        const size = view.getUint32(0, true);
        const type = view.getUint8(4);
        const tag = view.getUint16(5, true);
        const payload = bytes.slice(7, size);
        const pending = this.pending.get(tag);
        if (!pending) return;
        this.pending.delete(tag);
        if (type === P9_RLERROR) {
          const errno = new Reader(payload).u32();
          pending.reject(new Error(pending.name + " failed with errno " + errno));
          return;
        }
        appendLog("<- " + type + " for " + pending.name);
        pending.resolve({ type, tag, payload });
      }

      async version() {
        const payload = new Writer();
        payload.u32(131072);
        payload.string(P9_VERSION);
        await this.rpc(P9_TVERSION, payload.done());
      }

      async attach() {
        const payload = new Writer();
        payload.u32(this.rootFid);
        payload.u32(P9_NOFID);
        payload.string("browser");
        payload.string("");
        payload.u32(0);
        await this.rpc(P9_TATTACH, payload.done());
      }

      async walk(oldFid, parts) {
        const fid = this.allocFid();
        const payload = new Writer();
        payload.u32(oldFid);
        payload.u32(fid);
        payload.u16(parts.length);
        for (const part of parts) payload.string(part);
        const response = await this.rpc(P9_TWALK, payload.done());
        const reader = new Reader(response.payload);
        const qids = reader.u16();
        if (qids !== parts.length) throw new Error("walk resolved " + qids + " of " + parts.length + " path parts");
        return fid;
      }

      async open(fid, flags) {
        const payload = new Writer();
        payload.u32(fid);
        payload.u32(flags);
        await this.rpc(P9_TLOPEN, payload.done());
      }

      async create(parentFid, name) {
        const payload = new Writer();
        payload.u32(parentFid);
        payload.string(name);
        payload.u32(O_RDWR | O_TRUNC);
        payload.u32(MODE_FILE);
        payload.u32(0);
        await this.rpc(P9_TLCREATE, payload.done());
      }

      async symlink(parentFid, name, target) {
        const payload = new Writer();
        payload.u32(parentFid);
        payload.string(name);
        payload.string(target);
        payload.u32(0);
        await this.rpc(P9_TSYMLINK, payload.done());
      }

      async read(fid, offset, count) {
        const payload = new Writer();
        payload.u32(fid);
        payload.u64(offset);
        payload.u32(count);
        const response = await this.rpc(P9_TREAD, payload.done());
        const reader = new Reader(response.payload);
        return reader.data(reader.u32());
      }

      async write(fid, offset, data) {
        const payload = new Writer();
        payload.u32(fid);
        payload.u64(offset);
        payload.u32(data.length);
        payload.data(data);
        const response = await this.rpc(P9_TWRITE, payload.done());
        return new Reader(response.payload).u32();
      }

      async readdirPage(fid, offset) {
        const payload = new Writer();
        payload.u32(fid);
        payload.u64(offset);
        payload.u32(65536);
        const response = await this.rpc(P9_TREADDIR, payload.done());
        const reader = new Reader(response.payload);
        const entries = [];
        const dirBytes = new Reader(reader.data(reader.u32()));
        while (dirBytes.remaining() > 0) {
          const qid = dirBytes.qid();
          const offset = dirBytes.u64();
          const type = dirBytes.u8();
          const name = dirBytes.string();
          entries.push({ name, type, qid, offset });
        }
        return entries;
      }

      async readdir(fid) {
        const entries = [];
        let offset = 0n;
        while (true) {
          const page = await this.readdirPage(fid, offset);
          if (page.length === 0) break;
          entries.push(...page);
          const nextOffset = page[page.length - 1].offset;
          if (nextOffset <= offset) break;
          offset = nextOffset;
        }
        return entries;
      }

      async clunk(fid) {
        const payload = new Writer();
        payload.u32(fid);
        await this.rpc(P9_TCLUNK, payload.done());
      }

      async rename(oldPath, newPath) {
        const oldParent = await this.walkPath(parentPath(oldPath));
        const newParent = await this.walkPath(parentPath(newPath));
        try {
          const payload = new Writer();
          payload.u32(oldParent);
          payload.string(baseName(oldPath));
          payload.u32(newParent);
          payload.string(baseName(newPath));
          await this.rpc(P9_TRENAMEAT, payload.done());
        } finally {
          await this.clunk(oldParent).catch(() => {});
          await this.clunk(newParent).catch(() => {});
        }
      }

      async remove(path) {
        const parent = await this.walkPath(parentPath(path));
        try {
          const payload = new Writer();
          payload.u32(parent);
          payload.string(baseName(path));
          payload.u32(0);
          await this.rpc(P9_TUNLINKAT, payload.done());
        } finally {
          await this.clunk(parent).catch(() => {});
        }
      }

      async list(path) {
        const fid = await this.walkPath(path);
        try {
          await this.open(fid, O_RDONLY);
          return await this.readdir(fid);
        } finally {
          await this.clunk(fid).catch(() => {});
        }
      }

      async readText(path) {
        const fid = await this.walkPath(path);
        try {
          await this.open(fid, O_RDONLY);
          return text.decode(await this.read(fid, 0, 65536));
        } finally {
          await this.clunk(fid).catch(() => {});
        }
      }

      async readlink(path) {
        const fid = await this.walkPath(path);
        try {
          const payload = new Writer();
          payload.u32(fid);
          const response = await this.rpc(P9_TREADLINK, payload.done());
          return new Reader(response.payload).string();
        } finally {
          await this.clunk(fid).catch(() => {});
        }
      }

      async writeText(path, value) {
        const parent = await this.walkPath(parentPath(path));
        try {
          await this.create(parent, baseName(path));
          await this.write(parent, 0, utf8.encode(value));
        } finally {
          await this.clunk(parent).catch(() => {});
        }
      }

      async symlinkPath(target, linkPath) {
        const parent = await this.walkPath(parentPath(linkPath));
        try {
          await this.symlink(parent, baseName(linkPath), target);
        } finally {
          await this.clunk(parent).catch(() => {});
        }
      }

      async walkPath(path) {
        return await this.walk(this.rootFid, splitPath(path));
      }

      allocFid() { return this.nextFid++; }
    }

    function splitPath(path) {
      return path.split("/").filter(Boolean);
    }

    function parentPath(path) {
      const parts = splitPath(path);
      parts.pop();
      return "/" + parts.join("/");
    }

    function baseName(path) {
      const parts = splitPath(path);
      const name = parts.pop();
      if (!name) throw new Error("path must name an entry");
      return name;
    }

    function joinPath(parent, name) {
      return (parent === "/" ? "" : parent) + "/" + name;
    }

    function appendLog(line) {
      logOutput.textContent += line + "\n";
      logOutput.scrollTop = logOutput.scrollHeight;
    }

    function setConnected(connected) {
      for (const key of ["refresh", "read", "write", "rename", "remove"]) {
        buttons[key].disabled = !connected;
      }
    }

    async function run(label, operation) {
      try {
        status.textContent = label;
        await operation();
        status.textContent = "Connected";
      } catch (error) {
        status.textContent = error.message;
        appendLog("!! " + error.message);
      }
    }

    async function refreshEntries() {
      const names = await client.list(cwd.value || "/");
      entries.textContent = "";
      for (const entry of names) {
        const button = document.createElement("button");
        button.textContent = entry.name;
        button.addEventListener("click", () => {
          pathInput.value = joinPath(cwd.value || "/", entry.name);
        });
        entries.append(button);
      }
    }

    const discovery = await fetch("/.well-known/wanix.json", { cache: "no-store" }).then(response => response.json());
    discoveryOutput.textContent = JSON.stringify(discovery, null, 2);

    buttons.connect.addEventListener("click", () => run("Connecting", async () => {
      client = new P9Client(discovery.routes.p9.websocket);
      await client.connect();
      window.wanixP9 = client;
      setConnected(true);
      await refreshEntries();
    }));

    buttons.refresh.addEventListener("click", () => run("Listing", refreshEntries));
    buttons.read.addEventListener("click", () => run("Reading", async () => {
      editor.value = await client.readText(pathInput.value);
    }));
    buttons.write.addEventListener("click", () => run("Writing", async () => {
      await client.writeText(pathInput.value, editor.value);
      await refreshEntries();
    }));
    buttons.rename.addEventListener("click", () => run("Renaming", async () => {
      await client.rename(pathInput.value, renameTo.value);
      pathInput.value = renameTo.value;
      await refreshEntries();
    }));
    buttons.remove.addEventListener("click", () => run("Deleting", async () => {
      await client.remove(pathInput.value);
      await refreshEntries();
    }));
  </script>
</body>
</html>
"##,
    )
}

fn workbench_fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: workbench_fs9p_bundle_html().into_bytes(),
    }
}

fn workbench_fs9p_bundle_html() -> String {
    String::from(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Wanix Rust workbench</title>
  <style>
    html, body, #workbench { height: 100%; margin: 0; }
    body { overflow: hidden; color: #f5f5f5; background: #181818; font: 13px system-ui, sans-serif; }
    #workbench { width: 100%; }
    #status {
      position: fixed;
      z-index: 10;
      top: 10px;
      left: 10px;
      max-width: min(560px, calc(100vw - 20px));
      box-sizing: border-box;
      padding: 8px 10px;
      border: 1px solid #3f3f46;
      background: rgba(24, 24, 24, .92);
      color: #f5f5f5;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    body.ready #status { display: none; }
    body.error #status { border-color: #b85f5f; color: #ffd8d8; }
  </style>
</head>
<body>
  <div id="workbench" role="main"></div>
  <div id="status">Loading workbench assets...</div>
  <script>
    const statusEl = document.querySelector("#status");
    const container = document.querySelector("#workbench");
    const params = new URLSearchParams(location.search);
    const extensionId = params.get("extension") || "wanix.workbench";
    const workspaceUri = params.get("workspace") || "wanix:/";
    const rawAssets = params.get("assets") || "/workbench/";
    const discoveryUrl = new URL("/.well-known/wanix.json", location.href).href;
    const extensionRoot = new URL(rawAssets.replace(/\/?$/, "/"), location.href);
    const codeDir = new URL("code/", extensionRoot);
    const outDir = new URL("out/", codeDir);
    const outRoot = outDir.href.replace(/\/?$/, "");

    function showStatus(message) {
      statusEl.textContent = message;
    }

    function showError(error) {
      const message = error && error.stack ? error.stack : String(error);
      document.body.classList.remove("ready");
      document.body.classList.add("error");
      showStatus(message);
      console.error(error);
    }

    function loadScript(src) {
      return new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = src;
        script.onload = resolve;
        script.onerror = () => reject(new Error("failed to load " + src));
        document.head.appendChild(script);
      });
    }

    function loadStylesheet(href) {
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      document.head.appendChild(link);
    }

    function waitForAmdRequire() {
      return new Promise((resolve, reject) => {
        const started = performance.now();
        const poll = () => {
          const amdRequire = globalThis.require;
          if (amdRequire && typeof amdRequire.config === "function") {
            resolve(amdRequire);
            return;
          }
          if (performance.now() - started > 5000) {
            reject(new Error("VS Code AMD loader did not initialize"));
            return;
          }
          setTimeout(poll, 0);
        };
        poll();
      });
    }

    function createWorkbench(amdRequire, discovery) {
      return new Promise((resolve, reject) => {
        amdRequire(["vs/workbench/workbench.web.main"], async (wb) => {
          try {
            const workbenchConfig = { discoveryUrl };
            if (discovery.routes?.p9?.websocket) {
              workbenchConfig.p9 = discovery.routes.p9;
            }
            if (discovery.routes?.qjsShell?.websocket) {
              workbenchConfig.qjsShellUrl = discovery.routes.qjsShell.websocket;
              workbenchConfig.term = params.has("term");
            }
            if (discovery.services?.task && discovery.services?.term) {
              workbenchConfig.ns = {
                task: discovery.services.task,
                term: discovery.services.term
              };
              workbenchConfig.qjsTask = discovery.services.drivers?.includes("qjs") || false;
              workbenchConfig.shell = {
                cmd: params.get("cmd") || "",
                type: params.get("type") || "noop",
                wd: params.get("wd") || "."
              };
              workbenchConfig.term = params.has("term");
              workbenchConfig.raw = params.has("raw");
            }
            const activationChannel = new MessageChannel();
            activationChannel.port2.onmessage = (event) => {
              if (event.data?.type === "_port" && event.data.port) {
                event.data.port.postMessage({ config: workbenchConfig });
              }
            };
            const config = {
              additionalBuiltinExtensions: [wb.URI.parse(extensionRoot.href)],
              configurationDefaults: {
                "window.commandCenter": false,
                "workbench.layoutControl.enabled": false,
                "workbench.startupEditor": "none",
                "workbench.tips.enabled": false,
                "workbench.welcomePage.walkthroughs.openOnInstall": false,
                "editor.minimap.enabled": false,
                "explorer.confirmDelete": false,
                "explorer.confirmDragAndDrop": false
              },
              developmentOptions: { logLevel: params.has("debug") ? 2 : 0 },
              productConfiguration: {
                extensionEnabledApiProposals: { [extensionId]: ["ipc", "fileSearchProvider", "textSearchProvider"] }
              },
              workspaceProvider: {
                trusted: true,
                workspace: { folderUri: wb.URI.parse(workspaceUri) },
                open(workspace) {
                  const folder = workspace?.folderUri?.toString?.() || workspace?.workspaceUri?.toString?.();
                  if (!folder) return Promise.resolve(false);
                  const next = new URL(location.href);
                  next.searchParams.set("workspace", folder);
                  location.href = next.href;
                  return Promise.resolve(true);
                }
              },
              messagePorts: new Map([[extensionId, activationChannel.port1]])
            };
            globalThis.wanixWorkbench = {
              activationPort: activationChannel.port2,
              assets: extensionRoot.href,
              config: workbenchConfig,
              discovery,
              workspace: workspaceUri
            };
            await wb.create(container, config);
            document.body.classList.add("ready");
            resolve();
          } catch (error) {
            reject(error);
          }
        }, reject);
      });
    }

    (async () => {
      showStatus("Fetching Wanix discovery...");
      const discoveryResponse = await fetch(discoveryUrl, { cache: "no-store" });
      if (!discoveryResponse.ok) {
        throw new Error("failed to fetch discovery: HTTP " + discoveryResponse.status);
      }
      const discovery = await discoveryResponse.json();
      showStatus("Opening " + workspaceUri + " through " + discovery.routes.p9.websocket);
      loadStylesheet(new URL("vs/workbench/workbench.web.main.css", outDir).href);
      await loadScript(new URL("nls.messages.js", outDir).href);
      await loadScript(new URL("vs/loader.js", outDir).href);
      const amdRequire = await waitForAmdRequire();
      amdRequire.config({ baseUrl: outRoot });
      await createWorkbench(amdRequire, discovery);
    })().catch(showError);
  </script>
</body>
</html>
"##,
    )
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

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

struct StaticResponse {
    status: HttpStatus,
    content_type: &'static str,
    body: Vec<u8>,
}

impl StaticResponse {
    fn plain(status: HttpStatus, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {}\r\n\
             Content-Length: {}\r\n\
             Content-Type: {}\r\n\
             Cross-Origin-Opener-Policy: same-origin\r\n\
             Cross-Origin-Embedder-Policy: require-corp\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\
             \r\n",
            self.status.status_line(),
            self.body.len(),
            self.content_type
        )
        .into_bytes();
        response.extend_from_slice(&self.body);
        response
    }
}

#[derive(Copy, Clone)]
enum HttpStatus {
    Ok,
    BadRequest,
    Conflict,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotImplemented,
}

impl HttpStatus {
    fn status_line(self) -> &'static str {
        match self {
            Self::Ok => "200 OK",
            Self::BadRequest => "400 Bad Request",
            Self::Conflict => "409 Conflict",
            Self::Forbidden => "403 Forbidden",
            Self::NotFound => "404 Not Found",
            Self::MethodNotAllowed => "405 Method Not Allowed",
            Self::NotImplemented => "501 Not Implemented",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::BadRequest => "bad request",
            Self::Conflict => "conflict",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not found",
            Self::MethodNotAllowed => "method not allowed",
            Self::NotImplemented => "not implemented",
        }
    }
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
mod tests {
    use std::fs;
    use std::io::{self, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use tungstenite::{Message, connect, stream::MaybeTlsStream};
    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLERROR, P9_RLINK, P9_RLOPEN, P9_RREAD, P9_RREADLINK,
        P9_RREMOVE, P9_RRENAME, P9_RSETATTR, P9_RSYMLINK, P9_RVERSION, P9_RWALK, P9_RWALKGETATTR,
        P9_RWRITE, P9_SETATTR_GID, P9_SETATTR_UID, P9_VERSION_9P2000_L,
        P9_VERSION_9P2000_L_GOOGLE_2, P9Frame, P9SetAttr, p9_decode_rgetattr, p9_decode_rlerror,
        p9_decode_rlink, p9_decode_rread, p9_decode_rreadlink, p9_decode_rremove,
        p9_decode_rrename, p9_decode_rwalk, p9_decode_rwalkgetattr, p9_decode_rwrite, p9_tattach,
        p9_tauth, p9_tgetattr, p9_tlink, p9_tlopen, p9_tmknod, p9_tread, p9_treadlink, p9_tremove,
        p9_trename, p9_tsetattr, p9_tsymlink, p9_tversion, p9_twalk, p9_twalkgetattr, p9_twrite,
        p9_txattrcreate, p9_txattrwalk,
    };

    use super::*;

    const EBADF: u32 = 9;
    const ENOSYS: u32 = 38;
    const P9_O_WRONLY: u32 = 0o1;
    const P9_O_RDWR: u32 = 0o2;
    const EOPNOTSUPP: u32 = 95;

    #[test]
    fn parse_serve_uses_go_like_defaults_and_options() {
        let default_command = parse_serve_command(&[]).unwrap();
        assert_eq!(default_command.root_path, PathBuf::from("."));
        assert_eq!(default_command.addr, DEFAULT_SERVE_ADDR);
        assert_eq!(default_command.bundle, None);
        assert!(!default_command.wanix_services);
        assert!(!default_command.once);

        let command = parse_serve_command(&[
            OsString::from("examples"),
            OsString::from("--listen"),
            OsString::from(":7654"),
            OsString::from("--bundle"),
            OsString::from("vm-workbench"),
            OsString::from("--wanix-services"),
            OsString::from("--once"),
        ])
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("examples"));
        assert_eq!(command.addr, "0.0.0.0:7654");
        assert_eq!(command.bundle, Some("vm-workbench".to_owned()));
        assert!(command.wanix_services);
        assert!(command.once);

        let duplicate_dir =
            parse_serve_command(&[OsString::from("examples"), OsString::from("dist")]).unwrap_err();
        assert!(duplicate_dir.to_string().contains("only one directory"));
    }

    #[test]
    fn serve_once_returns_static_file_with_browser_isolation_headers() {
        let root = temp_dir("wanix-cli-serve-http");
        fs::write(root.join("index.html"), b"wanix serve").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.contains("wanix-rust serve: listening on http://127.0.0.1:"),
            "{stderr}"
        );
        assert!(stderr.contains("files with Wanix overlay"), "{stderr}");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Cross-Origin-Opener-Policy: same-origin\r\n"),
            "{response}"
        );
        assert!(
            response.contains("Cross-Origin-Embedder-Policy: require-corp\r\n"),
            "{response}"
        );
        assert!(
            response.contains("Access-Control-Allow-Origin: *\r\n"),
            "{response}"
        );
        assert!(response.ends_with("wanix serve"), "{response}");
    }

    #[test]
    fn serve_once_returns_mjs_static_file_as_javascript() {
        let root = temp_dir("wanix-cli-serve-mjs");
        fs::write(root.join("client.mjs"), b"export const ok = true;\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(addr, b"GET /client.mjs HTTP/1.1\r\nHost: localhost\r\n\r\n");
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: text/javascript; charset=utf-8\r\n"),
            "{response}"
        );
        assert!(
            response.ends_with("export const ok = true;\n"),
            "{response}"
        );
    }

    #[test]
    fn serve_once_reports_bundle_url_when_configured() {
        let root = temp_dir("wanix-cli-serve-bundle");
        fs::write(root.join("index.html"), b"wanix serve bundle").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(addr, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        assert!(
            String::from_utf8(response)
                .unwrap()
                .ends_with("wanix serve bundle")
        );
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.contains("bundle available at http://127.0.0.1:"),
            "{stderr}"
        );
        assert!(stderr.contains("/?bundle=vm-workbench"), "{stderr}");
    }

    #[test]
    fn serve_once_returns_direct_v86_bundle_page() {
        let root = temp_dir("wanix-cli-serve-direct-v86");
        fs::write(root.join("index.html"), b"static index").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /?bundle=direct-v86&bzimage=/boot/kernel HTTP/1.1\r\n\
              Host: demo.local:7654\r\n\
              \r\n",
        );
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("/?bundle=direct-v86"), "{stderr}");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: text/html; charset=utf-8\r\n"),
            "{response}"
        );
        assert!(
            response.contains("fetch(\"/.well-known/wanix.json\""),
            "{response}"
        );
        assert!(
            response.contains("const v86Assets = discovery.v86?.assets || {}"),
            "{response}"
        );
        assert!(
            response.contains("const DEFAULT_KERNEL_URL = \"/boot/bzImage\""),
            "{response}"
        );
        assert!(
            response.contains("const v86Boot = discovery.v86?.boot || {}"),
            "{response}"
        );
        assert!(
            response.contains("void loadRootfsHandoff(discovery.routes?.rootfs || {})"),
            "{response}"
        );
        assert!(response.contains("route = route || {}"), "{response}");
        assert!(
            response.contains("window.wanixRootfsRoute = route"),
            "{response}"
        );
        assert!(
            response.contains("window.wanixRootfsHandoff = null"),
            "{response}"
        );
        assert!(
            response.contains("rootfsHandoffCommands = null"),
            "{response}"
        );
        assert!(response.contains("copyQemu.disabled = true"), "{response}");
        assert!(response.contains("copyServe.disabled = true"), "{response}");
        assert!(
            response.contains("window.wanixRootfsHandoff = handoff"),
            "{response}"
        );
        assert!(response.contains("rootfsHandoffCommands = {"), "{response}");
        assert!(response.contains("qemuArgv,"), "{response}");
        assert!(response.contains("qemuCommand:"), "{response}");
        assert!(response.contains("serveArgv,"), "{response}");
        assert!(response.contains("serveCommand:"), "{response}");
        assert!(
            response.contains("function shellCommand(argv)"),
            "{response}"
        );
        assert!(
            response.contains("function shellQuoteArg(value)"),
            "{response}"
        );
        assert!(
            response.contains("await navigator.clipboard.writeText(command)"),
            "{response}"
        );
        assert!(
            response.contains("Rootfs handoff fetch failed: "),
            "{response}"
        );
        assert!(
            response.contains(
                "const { V86 } = await import(v86Assets.module || \"/v86/lib/libv86.mjs\")"
            ),
            "{response}"
        );
        assert!(
            response.contains("kernel.value = v86Boot.kernel || DEFAULT_KERNEL_URL"),
            "{response}"
        );
        assert!(
            response.contains("if (v86Boot.initrd) initrd.value = v86Boot.initrd"),
            "{response}"
        );
        assert!(
            response.contains("filesystem: { proxy_url: proxyUrl }"),
            "{response}"
        );
        assert!(
            response.contains(
                "const DEFAULT_CMDLINE = \"console=hvc0 init=/bin/init rw root=host9p rootfstype=9p"
            ),
            "{response}"
        );
        assert!(
            response.contains("cmdline.value = discovery.v86?.defaultCmdline || DEFAULT_CMDLINE"),
            "{response}"
        );
        assert!(
            response
                .contains("if (params.get(\"p9-msize\")) cmdline.value = cmdline.value.replace"),
            "{response}"
        );
        assert!(
            response
                .contains("if (params.get(\"cmdline\")) cmdline.value = params.get(\"cmdline\")"),
            "{response}"
        );
        assert!(
            response.contains("if (params.get(\"append\")) cmdline.value = [cmdline.value, params.get(\"append\")]"),
            "{response}"
        );
        assert!(response.contains("cmdline: cmdline.value"), "{response}");
        assert!(response.contains("virtio_console: true"), "{response}");
        assert!(
            response.contains("bzimage_initrd_from_filesystem: false"),
            "{response}"
        );
        assert!(response.contains("autostart: true"), "{response}");
        assert!(
            response.contains("memory_size: discovery.v86?.memorySize || DEFAULT_MEMORY_SIZE"),
            "{response}"
        );
        assert!(
            response.contains("wasm_path: v86Assets.wasm || \"/v86/bundle/v86.wasm\""),
            "{response}"
        );
        assert!(
            response.contains("bios: { url: v86Assets.bios || \"/v86/bundle/seabios.bin\" }"),
            "{response}"
        );
        assert!(
            response.contains("<div id=\"screen\"><canvas></canvas><div></div></div>"),
            "{response}"
        );
        assert!(
            response.contains("<textarea id=\"serial\" spellcheck=\"false\" readonly></textarea>"),
            "{response}"
        );
        assert!(
            response.contains("<pre id=\"boot-log\" aria-live=\"polite\"></pre>"),
            "{response}"
        );
        assert!(
            response.contains("<pre id=\"rootfs\" aria-live=\"polite\"></pre>"),
            "{response}"
        );
        assert!(
            response.contains("<label for=\"rootfs\">rootfs handoff</label>"),
            "{response}"
        );
        assert!(
            response.contains("<button id=\"copy-qemu\" disabled>Copy QEMU command</button>"),
            "{response}"
        );
        assert!(
            response.contains("<button id=\"copy-serve\" disabled>Copy serve command</button>"),
            "{response}"
        );
        assert!(
            response.contains("const hvc0 = document.querySelector(\"#serial\")"),
            "{response}"
        );
        assert!(
            response.contains("window.wanixV86BootLog = []"),
            "{response}"
        );
        assert!(response.contains("function startVm()"), "{response}");
        assert!(
            response.contains("if (params.get(\"autostart\") === \"1\") startVm()"),
            "{response}"
        );
        assert!(response.contains("connectBootStatus(vm)"), "{response}");
        assert!(
            response.contains("vm.add_listener(\"download-progress\""),
            "{response}"
        );
        assert!(
            response.contains("vm.add_listener(\"emulator-started\""),
            "{response}"
        );
        assert!(
            response.contains("vm.add_listener(\"virtio-console0-output-bytes\", appendHvc0)"),
            "{response}"
        );
        assert!(
            response.contains("vm.bus.send(\"virtio-console0-input-bytes\", bytes)"),
            "{response}"
        );
        assert!(response.contains("control === \"c\" ? 3 : 4"), "{response}");
        assert!(
            response.contains("window.wanixV86SendHvc0 = input =>"),
            "{response}"
        );
        assert!(
            response.contains("if (!vm) throw new Error(\"v86 is not started\")"),
            "{response}"
        );
        assert!(
            response.contains("return hvc0Encoder.encode(String(input))"),
            "{response}"
        );
        assert!(
            response.contains("vm.bus.send(\"virtio-console0-resize\", [columns, rows])"),
            "{response}"
        );
        assert!(
            response.contains("vm.add_listener(\"emulator-ready\", resize)"),
            "{response}"
        );
        assert!(
            response.contains("window.addEventListener(\"unhandledrejection\""),
            "{response}"
        );
        assert!(response.contains("window.wanixV86 = vm"), "{response}");
        assert!(response.contains("connectHvc0(vm)"), "{response}");
        assert!(!response.contains("static index"), "{response}");
    }

    #[test]
    fn serve_once_returns_fs9p_bundle_page() {
        let root = temp_dir("wanix-cli-serve-fs9p");
        fs::write(root.join("index.html"), b"static index").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(FS9P_BUNDLE.to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /?bundle=fs9p HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        );
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("/?bundle=fs9p"), "{stderr}");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: text/html; charset=utf-8\r\n"),
            "{response}"
        );
        assert!(
            response.contains("fetch(\"/.well-known/wanix.json\""),
            "{response}"
        );
        assert!(
            response.contains("new P9Client(discovery.routes.p9.websocket)"),
            "{response}"
        );
        assert!(response.contains("new WebSocket(this.url)"), "{response}");
        assert!(
            response.contains("this.socket.binaryType = \"arraybuffer\""),
            "{response}"
        );
        for operation in [
            "Tversion",
            "Tattach",
            "Twalk",
            "Tlopen",
            "Tlcreate",
            "Tsymlink",
            "Treadlink",
            "Tread",
            "Twrite",
            "Treaddir",
            "Trenameat",
            "Tunlinkat",
        ] {
            assert!(
                response.contains(operation),
                "{operation} missing from {response}"
            );
        }
        assert!(response.contains("async readText(path)"), "{response}");
        assert!(
            response.contains("async writeText(path, value)"),
            "{response}"
        );
        assert!(response.contains("async readlink(path)"), "{response}");
        assert!(
            response.contains("async symlinkPath(target, linkPath)"),
            "{response}"
        );
        assert!(
            response.contains("async rename(oldPath, newPath)"),
            "{response}"
        );
        assert!(response.contains("async remove(path)"), "{response}");
        assert!(
            response.contains("async readdirPage(fid, offset)"),
            "{response}"
        );
        assert!(response.contains("payload.u64(offset)"), "{response}");
        assert!(response.contains("entries.push(...page)"), "{response}");
        assert!(response.contains("window.wanixP9 = client"), "{response}");
        assert!(!response.contains("static index"), "{response}");
        assert!(!response.contains("globalThis.Wanix"), "{response}");
    }

    #[test]
    fn serve_once_returns_workbench_fs9p_bundle_page() {
        let root = temp_dir("wanix-cli-serve-workbench-fs9p");
        fs::write(root.join("index.html"), b"static index").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /?bundle=workbench-fs9p HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        );
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("/?bundle=workbench-fs9p"), "{stderr}");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: text/html; charset=utf-8\r\n"),
            "{response}"
        );
        assert!(
            response.contains(
                "const discoveryUrl = new URL(\"/.well-known/wanix.json\", location.href).href"
            ),
            "{response}"
        );
        assert!(response.contains("fetch(discoveryUrl"), "{response}");
        assert!(
            response.contains("new URL(\"vs/loader.js\", outDir)"),
            "{response}"
        );
        assert!(
            response.contains("new URL(\"vs/workbench/workbench.web.main.css\", outDir)"),
            "{response}"
        );
        assert!(
            response.contains("amdRequire.config({ baseUrl: outRoot })"),
            "{response}"
        );
        assert!(
            response.contains("[\"vs/workbench/workbench.web.main\"]"),
            "{response}"
        );
        assert!(
            response.contains("additionalBuiltinExtensions: [wb.URI.parse(extensionRoot.href)]"),
            "{response}"
        );
        assert!(
            response.contains("extensionEnabledApiProposals: { [extensionId]: [\"ipc\", \"fileSearchProvider\", \"textSearchProvider\"] }"),
            "{response}"
        );
        assert!(
            response.contains("const activationChannel = new MessageChannel()"),
            "{response}"
        );
        assert!(
            response.contains("const workbenchConfig = { discoveryUrl }"),
            "{response}"
        );
        assert!(
            response.contains("workbenchConfig.p9 = discovery.routes.p9"),
            "{response}"
        );
        assert!(
            response.contains("if (discovery.services?.task && discovery.services?.term)"),
            "{response}"
        );
        assert!(
            response.contains(
                "workbenchConfig.qjsTask = discovery.services.drivers?.includes(\"qjs\") || false"
            ),
            "{response}"
        );
        assert!(
            response.contains("if (discovery.routes?.qjsShell?.websocket)"),
            "{response}"
        );
        assert!(
            response.contains("workbenchConfig.qjsShellUrl = discovery.routes.qjsShell.websocket"),
            "{response}"
        );
        assert!(
            response.contains("event.data.port.postMessage({ config: workbenchConfig })"),
            "{response}"
        );
        assert!(response.contains("type: params.get(\"type\") || \"noop\""));
        assert!(response.contains("workbenchConfig.term = params.has(\"term\")"));
        assert!(
            response.contains("messagePorts: new Map([[extensionId, activationChannel.port1]])"),
            "{response}"
        );
        assert!(
            response.contains("workspace: { folderUri: wb.URI.parse(workspaceUri) }"),
            "{response}"
        );
        assert!(
            response.contains("next.searchParams.set(\"workspace\", folder)"),
            "{response}"
        );
        assert!(!response.contains("todo: handle openFolder"), "{response}");
        assert!(
            response.contains("const workspaceUri = params.get(\"workspace\") || \"wanix:/\""),
            "{response}"
        );
        assert!(
            response.contains("const rawAssets = params.get(\"assets\") || \"/workbench/\""),
            "{response}"
        );
        assert!(
            response.contains("globalThis.wanixWorkbench ="),
            "{response}"
        );
        assert!(!response.contains("event.data.wanix"), "{response}");
        assert!(!response.contains("new WanixHandle"), "{response}");
        assert!(!response.contains("globalThis.Wanix"), "{response}");
        assert!(!response.contains("static index"), "{response}");
    }

    #[test]
    fn serve_once_returns_direct_v86_embedded_asset_over_static_collision() {
        let root = temp_dir("wanix-cli-serve-direct-v86-assets");
        fs::create_dir_all(root.join("v86/bundle")).unwrap();
        fs::write(root.join("v86/bundle/v86.wasm"), b"not wasm").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /v86/bundle/v86.wasm HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&response[..header_end]);
        let body = &response[header_end..];
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
        assert!(
            headers.contains("Content-Type: application/wasm\r\n"),
            "{headers}"
        );
        assert!(
            body.starts_with(b"\0asm"),
            "body did not start with wasm magic"
        );
        assert_ne!(body, b"not wasm");
    }

    #[test]
    fn direct_v86_embedded_asset_table_serves_browser_runtime_assets() {
        let root = temp_dir("wanix-cli-direct-v86-asset-table");
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:0".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        for (path, content_type, prefix) in [
            (
                "v86/lib/libv86.mjs",
                "text/javascript; charset=utf-8",
                Some(b";let module".as_slice()),
            ),
            (
                "v86/lib/mod.js",
                "text/javascript; charset=utf-8",
                Some(b"export { V86 }".as_slice()),
            ),
            (
                "v86/lib/offscreen.js",
                "text/javascript; charset=utf-8",
                None,
            ),
            ("v86/bundle/v86.wasm", "application/wasm", Some(b"\0asm")),
            ("v86/bundle/seabios.bin", "application/octet-stream", None),
            ("v86/bundle/vgabios.bin", "application/octet-stream", None),
        ] {
            let response = direct_v86_asset_response(&roots, Path::new(path)).unwrap();
            assert_eq!(response.status.status_line(), "200 OK");
            assert_eq!(response.content_type, content_type);
            assert!(!response.body.is_empty(), "{path} was empty");
            if let Some(prefix) = prefix {
                assert!(
                    response.body.starts_with(prefix),
                    "{path} body did not start with expected bytes"
                );
            }
        }

        let other_roots = ServeRoots::new(
            &root,
            "127.0.0.1:0".parse().unwrap(),
            Some("vm-workbench".to_owned()),
            false,
        )
        .unwrap();
        assert!(
            direct_v86_asset_response(&other_roots, Path::new("v86/bundle/v86.wasm")).is_none()
        );
    }

    #[test]
    fn serve_once_keeps_direct_v86_asset_paths_static_without_direct_bundle() {
        let root = temp_dir("wanix-cli-serve-static-v86-asset");
        fs::create_dir_all(root.join("v86/bundle")).unwrap();
        fs::write(root.join("v86/bundle/v86.wasm"), b"static wasm").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /v86/bundle/v86.wasm HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&response[..header_end]);
        let body = &response[header_end..];
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
        assert!(
            headers.contains("Content-Type: application/wasm\r\n"),
            "{headers}"
        );
        assert_eq!(body, b"static wasm");
    }

    #[test]
    fn serve_once_keeps_other_bundles_on_static_root() {
        let root = temp_dir("wanix-cli-serve-other-bundle");
        fs::write(root.join("index.html"), b"static bundle index").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /?bundle=direct-v86 HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(response.ends_with("static bundle index"), "{response}");
        assert!(!response.contains("filesystem: { proxy_url: proxyUrl }"));
    }

    #[test]
    fn serve_once_returns_well_known_discovery_document() {
        let root = temp_dir("wanix-cli-serve-discovery");
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("index.html"), b"wanix serve discovery").unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        fs::write(root.join("boot/initrd"), b"initrd").unwrap();
        write_boot_init(&root);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: application/json\r\n"),
            "{response}"
        );
        assert!(
            response.contains(
                "\"p9\":{\"websocket\":\"ws://demo.local:7654/.well-known/export9p\",\
                 \"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\",\
                 \"supportedProtocols\":[\"9P2000.L\",\"9P2000.L.Google.2\"]}"
            ),
            "{response}"
        );
        assert!(
            response.contains(
                "\"rootfs\":{\"url\":\"http://demo.local:7654/.well-known/rootfs.json\",\
                 \"kind\":\"wanix-rootfs.v1\",\"status\":\"available\",\"ready\":true}"
            ),
            "{response}"
        );
        assert!(
            response.contains(
                "\"ethernet\":{\"websocket\":\"ws://demo.local:7654/.well-known/ethernet\",\
                 \"status\":\"not-implemented\"}"
            ),
            "{response}"
        );
        assert!(
            response.contains("\"qjsShell\":{\"status\":\"disabled\"}"),
            "{response}"
        );
        assert!(
            response.contains("\"bundle\":\"vm-workbench\""),
            "{response}"
        );
        assert!(
            response.contains("\"assets\":{\"module\":\"/v86/lib/libv86.mjs\""),
            "{response}"
        );
        assert!(
            response.contains("\"mod\":\"/v86/lib/mod.js\""),
            "{response}"
        );
        assert!(
            response.contains("\"offscreen\":\"/v86/lib/offscreen.js\""),
            "{response}"
        );
        assert!(
            response.contains("\"wasm\":\"/v86/bundle/v86.wasm\""),
            "{response}"
        );
        assert!(
            response.contains("\"bios\":\"/v86/bundle/seabios.bin\""),
            "{response}"
        );
        assert!(
            response.contains("\"vgaBios\":\"/v86/bundle/vgabios.bin\""),
            "{response}"
        );
        assert!(
            response
                .contains("\"boot\":{\"kernel\":\"/boot/bzImage\",\"initrd\":\"/boot/initrd\",\"init\":\"/bin/init\",\"ready\":true}"),
            "{response}"
        );
        assert!(
            response.contains("\"defaultCmdline\":\"console=hvc0 init=/bin/init rw root=host9p"),
            "{response}"
        );
        assert!(response.contains("\"p9Msize\":131072"), "{response}");
        assert!(
            response.contains(
                "\"memorySize\":1073741824,\"vgaMemorySize\":8388608,\"virtioConsole\":true"
            ),
            "{response}"
        );
        assert!(response.contains("\"services\":null"), "{response}");
    }

    #[test]
    fn serve_once_returns_rootfs_handoff_manifest() {
        let root = temp_dir("wanix-cli-serve-rootfs-handoff");
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        fs::write(root.join("boot/initrd"), b"initrd").unwrap();
        write_boot_init(&root);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root.clone(),
            addr: addr.to_string(),
            bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
            wanix_services: true,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/rootfs.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let (headers, body) = http_response_parts(&response);
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
        assert!(
            headers.contains("Content-Type: application/json\r\n"),
            "{headers}"
        );
        let manifest: serde_json::Value = serde_json::from_slice(body).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let kernel = root.join("boot/bzImage");
        let initrd = root.join("boot/initrd");
        let init = root.join("bin/init");
        let qemu = &manifest["qemu"];
        let serve = &manifest["serveDirectV86"];

        assert_eq!(manifest["kind"], "wanix-rootfs.v1");
        assert_eq!(manifest["rootPath"], root.display().to_string());
        assert_eq!(manifest["kernelRoute"], "/boot/bzImage");
        assert_eq!(manifest["kernelPath"], kernel.display().to_string());
        assert_eq!(manifest["initRoute"], "/bin/init");
        assert_eq!(manifest["initPath"], init.display().to_string());
        assert_eq!(qemu["kind"], "wanix-qemu-virtio9p.v1");
        assert_eq!(qemu["rootPath"], root.display().to_string());
        assert_eq!(qemu["kernelPath"], kernel.display().to_string());
        assert_eq!(qemu["initrdPath"], initrd.display().to_string());
        assert_eq!(qemu["mountTag"], "host9p");
        assert_eq!(qemu["securityModel"], "mapped-xattr");
        assert_eq!(serve["bundle"], "direct-v86");
        assert_eq!(serve["wanixServices"], true);
        assert_eq!(serve["p9Msize"], 131072);
        assert_eq!(serve["argv"][0], "wanix-rust");
        assert_eq!(serve["argv"][1], "serve");
        assert_eq!(serve["argv"][2], root.display().to_string());
    }

    #[test]
    fn serve_rootfs_handoff_reports_unprepared_roots_and_reserves_route() {
        let root = temp_dir("wanix-cli-serve-rootfs-handoff-missing");
        fs::create_dir_all(root.join(".well-known")).unwrap();
        fs::write(root.join(".well-known/rootfs.json"), b"not static").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/rootfs.json HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 409 Conflict\r\n"),
            "{response}"
        );
        assert!(response.contains("missing a guest kernel"), "{response}");
        assert!(!response.contains("not static"), "{response}");
    }

    #[test]
    fn serve_rootfs_handoff_rejects_non_loopback_peers_without_paths() {
        let root = temp_dir("wanix-cli-serve-rootfs-handoff-remote");
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        write_boot_init(&root);
        let roots = ServeRoots::new(&root, "0.0.0.0:7654".parse().unwrap(), None, false).unwrap();
        let response = rootfs_handoff_response(&roots, "192.0.2.1:12345".parse().unwrap());
        let status = response.status.status_line();
        let body = String::from_utf8(response.body).unwrap();

        assert_eq!(status, "403 Forbidden");
        assert!(body.contains("loopback clients"), "{body}");
        assert!(!body.contains(&fs::canonicalize(root).unwrap().display().to_string()));
        assert!(!body.contains("bzImage"), "{body}");
    }

    #[test]
    fn serve_wanix_services_root_exports_task_and_terminal_services() {
        let root = temp_dir("wanix-cli-serve-services-root");
        fs::write(root.join("host.txt"), b"host file").unwrap();
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            true,
        )
        .unwrap();

        let discovery = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );
        assert!(
            discovery.contains(
                "\"services\":{\"task\":\"#task\",\"term\":\"#term\",\
                 \"drivers\":[\"noop\",\"qjs\"]}"
            ),
            "{discovery}"
        );
        let discovery_json: serde_json::Value = serde_json::from_str(&discovery).unwrap();
        let qjs_shell = &discovery_json["routes"]["qjsShell"];
        assert_eq!(
            qjs_shell["websocket"],
            "ws://demo.local:7654/.well-known/qjs-shell"
        );
        assert_eq!(qjs_shell["protocol"], "wanix-qjs-shell.v1");
        assert_eq!(qjs_shell["mode"], "raw-bytes");
        assert_eq!(qjs_shell["status"], "available");
        assert_eq!(qjs_shell["cwdQuery"], "cwd");
        assert_eq!(qjs_shell["defaultCwd"], ".");
        assert_eq!(
            qjs_shell["resize"][0],
            "{\"type\":\"resize\",\"columns\":COLS,\"rows\":ROWS}"
        );
        assert_eq!(qjs_shell["resize"][1], "resize COLS ROWS");
        assert_eq!(qjs_shell["exitMessage"], "{\"type\":\"exit\",\"code\":N}");
        assert_eq!(
            qjs_shell["terminalLifecycle"],
            "owned-resource-closed-on-session-close"
        );

        let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());
        let response = server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server
            .handle_frame(&p9_twalk(2, 1, 2, &["#term", "new"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 2);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(3, 2, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server.handle_frame(&p9_tread(4, 2, 0, 64)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"1\n");

        let response = server
            .handle_frame(&p9_twalk(5, 1, 3, &["#task", "self", "id"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(6, 3, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server.handle_frame(&p9_tread(7, 3, 0, 64)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"1\n");

        let response = server
            .handle_frame(&p9_twalk(8, 1, 4, &["#task", "new", "noop"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(9, 4, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server.handle_frame(&p9_tread(10, 4, 0, 64)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"2\n");
    }

    #[test]
    fn serve_wanix_services_terminal_winch_broadcasts_over_9p() {
        let root = temp_dir("wanix-cli-serve-services-winch");
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            true,
        )
        .unwrap();
        let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

        assert_eq!(
            server
                .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
                .unwrap()
                .message_type(),
            P9_RATTACH
        );
        assert_eq!(
            p9_read_file(&mut server, 1, 2, 100, &["#term", "new"]),
            b"1\n"
        );

        let response = server
            .handle_frame(&p9_twalk(110, 1, 3, &["#term", "1", "winch"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(111, 3, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );

        let response = server
            .handle_frame(&p9_twalk(120, 1, 4, &["#term", "1", "winch"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(121, 4, P9_O_RDWR))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server
            .handle_frame(&p9_twrite(122, 4, 0, b"100 40\n").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 7);

        let response = server.handle_frame(&p9_tread(130, 3, 0, 64)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"100 40\n");
    }

    #[test]
    fn serve_wanix_services_terminal_ctl_close_is_visible_over_9p() {
        let root = temp_dir("wanix-cli-serve-services-term-close");
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            true,
        )
        .unwrap();
        let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

        assert_eq!(
            server
                .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
                .unwrap()
                .message_type(),
            P9_RATTACH
        );
        assert_eq!(
            p9_read_file(&mut server, 1, 2, 100, &["#term", "new"]),
            b"1\n"
        );

        let response = server
            .handle_frame(&p9_twalk(110, 1, 3, &["#term", "1", "data"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(111, 3, P9_O_RDWR))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server
            .handle_frame(&p9_twrite(112, 3, 0, b"before close").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);

        let response = server
            .handle_frame(&p9_twalk(120, 1, 4, &["#term", "1", "ctl"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(121, 4, P9_O_WRONLY))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server
            .handle_frame(&p9_twrite(122, 4, 0, b"clo").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        let response = server
            .handle_frame(&p9_twrite(123, 3, 0, b"still open").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        let response = server
            .handle_frame(&p9_twrite(124, 4, 0, b"se\n").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);

        let response = server
            .handle_frame(&p9_twalk(140, 1, 6, &["#term", "1", "data"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        let response = server
            .handle_frame(&p9_twrite(150, 3, 0, b"after close").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn serve_wanix_services_can_start_qjs_task_through_9p_task_service() {
        let root = temp_dir("wanix-cli-serve-services-qjs-task");
        fs::write(
            root.join("main.js"),
            br##"
import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
std.out.puts("serve qjs task " + id + "\n");
std.writeFile("generated.txt", "generated by task " + id);
"##,
        )
        .unwrap();
        fs::write(root.join("stdout.txt"), b"").unwrap();
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            true,
        )
        .unwrap();
        let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

        assert_eq!(
            server
                .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
                .unwrap()
                .message_type(),
            P9_RATTACH
        );
        assert_eq!(
            p9_read_file(&mut server, 1, 2, 100, &["#task", "new", "qjs"]),
            b"2\n"
        );
        p9_write_file(&mut server, 1, 3, 110, &["#task", "2", "cmd"], b"main.js\n");
        p9_write_file(&mut server, 1, 4, 120, &["#task", "2", "dir"], b".\n");
        p9_write_file(
            &mut server,
            1,
            5,
            130,
            &["#task", "2", "ctl"],
            b"bind stdout.txt fd/1\n",
        );
        p9_write_file(&mut server, 1, 6, 140, &["#task", "2", "ctl"], b"start\n");

        assert_eq!(
            p9_read_file(&mut server, 1, 7, 150, &["#task", "2", "exit"]),
            b"0\n"
        );
        assert_eq!(
            fs::read(root.join("stdout.txt")).unwrap(),
            b"serve qjs task 2\n"
        );
        assert_eq!(
            fs::read(root.join("generated.txt")).unwrap(),
            b"generated by task 2"
        );
    }

    #[test]
    fn serve_discovery_falls_back_to_listener_address_without_host_header() {
        let root = temp_dir("wanix-cli-serve-discovery-no-host");
        let roots = ServeRoots::new(
            &root,
            "0.0.0.0:7654".parse().unwrap(),
            Some("quote\"bundle".to_owned()),
            false,
        )
        .unwrap();
        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );

        assert!(
            body.contains("\"websocket\":\"ws://localhost:7654/.well-known/export9p\""),
            "{body}"
        );
        assert!(body.contains("\"bundle\":\"quote\\\"bundle\""), "{body}");
        assert!(
            body.contains("\"defaultCmdline\":\"console=hvc0 init=/bin/init rw root=host9p"),
            "{body}"
        );

        let ipv6_roots =
            ServeRoots::new(&root, "[::1]:7654".parse().unwrap(), None, false).unwrap();
        let ipv6_body = serve_discovery_json(
            &ipv6_roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
            "[::1]:12345".parse().unwrap(),
        );
        assert!(
            ipv6_body.contains("\"websocket\":\"ws://[::1]:7654/.well-known/export9p\""),
            "{ipv6_body}"
        );
    }

    #[test]
    fn serve_discovery_finds_direct_v86_legacy_top_level_kernel() {
        let root = temp_dir("wanix-cli-serve-discovery-legacy-kernel");
        fs::write(root.join("bzImage"), b"kernel").unwrap();
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );

        assert!(
            body.contains(
                "\"boot\":{\"kernel\":\"/bzImage\",\"ready\":false,\"missing\":[\"/bin/init\"]}"
            ),
            "{body}"
        );
    }

    #[test]
    fn serve_discovery_reports_direct_v86_boot_readiness_gaps() {
        let root = temp_dir("wanix-cli-serve-discovery-boot-gaps");
        write_boot_init(&root);
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );

        assert!(
            body.contains(
                "\"boot\":{\"init\":\"/bin/init\",\"ready\":false,\"missing\":[\"/boot/bzImage\"]}"
            ),
            "{body}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn serve_discovery_requires_executable_direct_v86_init() {
        let root = temp_dir("wanix-cli-serve-discovery-non-executable-init");
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        fs::write(root.join("bin/init"), b"init").unwrap();
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );
        let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
        let boot = &discovery["v86"]["boot"];
        let rootfs = &discovery["routes"]["rootfs"];

        assert_eq!(boot["kernel"], "/boot/bzImage");
        assert!(boot["init"].is_null());
        assert_eq!(boot["ready"], false);
        assert_eq!(boot["missing"][0], "/bin/init");
        assert_eq!(rootfs["status"], "unprepared");
        assert_eq!(rootfs["ready"], false);
        assert_eq!(rootfs["missing"][0], "/bin/init");
    }

    #[test]
    fn serve_discovery_reports_rootfs_handoff_status() {
        let root = temp_dir("wanix-cli-serve-discovery-rootfs-status");
        write_boot_init(&root);
        let roots = ServeRoots::new(
            &root,
            "0.0.0.0:7654".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );
        let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
        let rootfs = &discovery["routes"]["rootfs"];
        assert_eq!(
            rootfs["url"],
            "http://demo.local:7654/.well-known/rootfs.json"
        );
        assert_eq!(rootfs["kind"], "wanix-rootfs.v1");
        assert_eq!(rootfs["status"], "unprepared");
        assert_eq!(rootfs["ready"], false);
        assert_eq!(rootfs["missing"][0], "/boot/bzImage");

        let remote_body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "192.0.2.1:12345".parse().unwrap(),
        );
        let remote_discovery: serde_json::Value = serde_json::from_str(&remote_body).unwrap();
        assert_eq!(remote_discovery["routes"]["rootfs"]["status"], "local-only");
    }

    #[test]
    fn serve_discovery_does_not_overpromise_invalid_rootfs_handoff() {
        let parent = temp_dir("wanix-cli-serve-discovery-rootfs-invalid");
        let root = parent.join("with,comma");
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        write_boot_init(&root);
        let roots = ServeRoots::new(
            &root,
            "127.0.0.1:7654".parse().unwrap(),
            Some(DIRECT_V86_BUNDLE.to_owned()),
            false,
        )
        .unwrap();

        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );
        let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
        let rootfs = &discovery["routes"]["rootfs"];
        assert_eq!(rootfs["status"], "invalid");
        assert_eq!(rootfs["ready"], false);
        assert!(
            rootfs["error"]
                .as_str()
                .unwrap()
                .contains("cannot contain ','"),
            "{rootfs}"
        );
        assert!(rootfs.get("missing").is_none(), "{rootfs}");
    }

    #[test]
    fn serve_discovery_ignores_unsafe_host_header() {
        let root = temp_dir("wanix-cli-serve-discovery-unsafe-host");
        let roots = ServeRoots::new(&root, "127.0.0.1:7654".parse().unwrap(), None, false).unwrap();
        let body = serve_discovery_json(
            &roots,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654/escape\r\n\r\n",
            "127.0.0.1:12345".parse().unwrap(),
        );

        assert!(
            body.contains("\"websocket\":\"ws://127.0.0.1:7654/.well-known/export9p\""),
            "{body}"
        );
    }

    #[test]
    fn serve_once_exports_9p_over_binary_websocket() {
        let root = temp_dir("wanix-cli-serve-ws");
        fs::write(root.join("hello.txt"), b"hello serve").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/")).unwrap().0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tgetattr(4, 2, u64::MAX),
            p9_tlopen(5, 2, 0),
            p9_tread(6, 2, 0, 11),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 6);
        socket.close(None).unwrap();
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RATTACH,
                P9_RWALK,
                P9_RGETATTR,
                P9_RLOPEN,
                P9_RREAD
            ]
        );
        let attr = p9_decode_rgetattr(&frames[3]).unwrap();
        assert_eq!(attr.size, 11);
        assert_eq!(attr.mode & 0o170000, 0o100000);
        assert_eq!(p9_decode_rread(&frames[5]).unwrap(), b"hello serve");
    }

    #[test]
    fn serve_concurrent_loop_serves_http_while_9p_websocket_stays_open() {
        let root = temp_dir("wanix-cli-serve-concurrent-http");
        fs::write(root.join("index.html"), b"wanix serve concurrent").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
            wanix_services: false,
            once: false,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code =
                run_serve_with_listener_for_connections(command, listener, &mut stderr, 2).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        socket
            .send(Message::binary(request_stream([p9_tversion(
                1,
                8192,
                P9_VERSION_9P2000_L,
            )
            .unwrap()])))
            .unwrap();
        let frames = read_binary_frames(&mut socket, 1);
        assert_eq!(frame_types(&frames), [P9_RVERSION]);

        let response = http_request(
            addr,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        );
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("\"websocket\":\"ws://demo.local:7654/.well-known/export9p\""),
            "{response}"
        );

        socket.close(None).unwrap();
        let (exit_code, stderr) = handle.join().unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        assert_eq!(exit_code, 0, "{stderr}");
    }

    #[test]
    fn serve_concurrent_loop_serves_two_9p_websockets() {
        let root = temp_dir("wanix-cli-serve-concurrent-9p");
        fs::write(root.join("hello.txt"), b"hello concurrent").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: false,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code =
                run_serve_with_listener_for_connections(command, listener, &mut stderr, 2).unwrap();
            (exit_code, stderr)
        });

        let mut first = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let mut second = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let requests = || {
            request_stream([
                p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
                p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
                p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
                p9_tlopen(4, 2, 0),
                p9_tread(5, 2, 0, 16),
            ])
        };
        first.send(Message::binary(requests())).unwrap();
        second.send(Message::binary(requests())).unwrap();

        let first_frames = read_binary_frames(&mut first, 5);
        let second_frames = read_binary_frames(&mut second, 5);
        first.close(None).unwrap();
        second.close(None).unwrap();
        let (exit_code, stderr) = handle.join().unwrap();
        let stderr = String::from_utf8(stderr).unwrap();

        assert_eq!(exit_code, 0, "{stderr}");
        for frames in [&first_frames, &second_frames] {
            assert_eq!(
                frame_types(frames),
                [P9_RVERSION, P9_RATTACH, P9_RWALK, P9_RLOPEN, P9_RREAD]
            );
            assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello concurrent");
        }
    }

    #[test]
    fn serve_once_exports_9p_on_well_known_export_path() {
        let root = temp_dir("wanix-cli-serve-export9p");
        fs::write(root.join("hello.txt"), b"hello export").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tsetattr(
                4,
                2,
                P9_SETATTR_UID | P9_SETATTR_GID,
                &P9SetAttr {
                    uid: 1000,
                    gid: 1001,
                    ..P9SetAttr::default()
                },
            ),
            p9_tgetattr(5, 2, u64::MAX),
            p9_tlopen(6, 2, 0),
            p9_tread(7, 2, 0, 12),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 7);
        socket.close(None).unwrap();
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RATTACH,
                P9_RWALK,
                P9_RSETATTR,
                P9_RGETATTR,
                P9_RLOPEN,
                P9_RREAD
            ]
        );
        let attr = p9_decode_rgetattr(&frames[4]).unwrap();
        assert_eq!(attr.uid, 1000);
        assert_eq!(attr.gid, 1001);
        assert_eq!(p9_decode_rread(&frames[6]).unwrap(), b"hello export");
    }

    #[test]
    #[cfg(unix)]
    fn serve_once_exports_symlink_readlink_over_direct_9p() {
        let root = temp_dir("wanix-cli-serve-export9p-symlink");
        fs::write(root.join("target.txt"), b"linked data").unwrap();
        let root_check = root.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_tsymlink(3, 1, "link.txt", "target.txt", 0).unwrap(),
            p9_twalk(4, 1, 2, &["link.txt"]).unwrap(),
            p9_treadlink(5, 2),
            p9_tlopen(6, 2, 0),
            p9_tread(7, 2, 0, 11),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 7);
        socket.close(None).unwrap();
        let (exit_code, stderr) = handle.join().unwrap();
        let stderr = String::from_utf8(stderr).unwrap();

        assert_eq!(exit_code, 0, "{stderr}");
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RATTACH,
                P9_RSYMLINK,
                P9_RWALK,
                P9_RREADLINK,
                P9_RLOPEN,
                P9_RREAD
            ]
        );
        assert_eq!(p9_decode_rreadlink(&frames[4]).unwrap(), "target.txt");
        assert_eq!(p9_decode_rread(&frames[6]).unwrap(), b"linked data");
        assert_eq!(
            fs::read_link(root_check.join("link.txt")).unwrap(),
            std::path::PathBuf::from("target.txt")
        );
    }

    #[test]
    fn serve_once_exports_qjs_shell_terminal_websocket() {
        let root = temp_dir("wanix-cli-serve-qjs-shell-ws");
        fs::create_dir(root.join("app")).unwrap();
        fs::write(root.join("app").join("inside.txt"), b"inside app\n").unwrap();
        fs::write(
            root.join("foreground-child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
if (count < 0) {
  throw new Error("stdin read failed: " + count);
}
const text = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
std.out.puts("served foreground stdin " + text.trimEnd() + "\n");
std.exit(0);
"##,
        )
        .unwrap();
        fs::write(
            root.join("terminal-child.js"),
            r#"import * as std from "qjs:std";

std.out.puts("served terminal child stdout\n");
std.err.puts("served terminal child stderr\n");
std.exit(0);
"#,
        )
        .unwrap();
        fs::write(
            root.join("child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts("served child task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("served child cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("served child argv " + scriptArgs.join("|") + "\n");
std.out.puts("served child stdin " + readStdin().trimEnd() + "\n");
std.out.puts("served child mode " + std.getenv("MODE") + "\n");
std.out.flush();
std.err.puts("served child stderr " + scriptArgs[1] + "\n");
std.err.flush();
std.exit(6);
"##,
        )
        .unwrap();
        fs::write(root.join("visible.txt"), b"served root").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            wanix_services: true,
            once: false,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code =
                run_serve_with_listener_for_connections(command, listener, &mut stderr, 1).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/qjs-shell?cwd=app"))
            .unwrap()
            .0;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(60)))
                .unwrap();
        }
        let initial = socket.read().unwrap();
        match initial {
            Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"shell task: 1\r\n$ "),
            other => panic!("expected initial terminal output, got {other:?}"),
        }

        socket
            .send(Message::binary(b"echo nope\x03echo yes\n".as_slice()))
            .unwrap();
        match socket.read().unwrap() {
            Message::Binary(bytes) => {
                assert_eq!(bytes.as_ref(), b"echo nope^C\r\n$ echo yes\r\nyes\r\n$ ")
            }
            other => panic!("expected Ctrl-C line cancellation output, got {other:?}"),
        }

        socket
            .send(Message::Text(
                r#"{"type":"resize","columns":100,"rows":40}"#.into(),
            ))
            .unwrap();
        socket
            .send(Message::binary(
                b"pwd\nls\ncat inside.txt\nstat inside.txt\nsize\nlater idle\n".as_slice(),
            ))
            .unwrap();
        match socket.read().unwrap() {
            Message::Binary(bytes) => assert_eq!(
                bytes.as_ref(),
                b"pwd\r\napp\r\n$ ls\r\ninside.txt\r\n$ cat inside.txt\r\ninside app\r\n$ stat inside.txt\r\ninside.txt type file mode 100000 size 11\r\n$ size\r\nsize 100 40\r\n$ later idle\r\nscheduled\r\n"
            ),
            other => panic!("expected scheduled terminal output, got {other:?}"),
        }
        match socket.read().unwrap() {
            Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"later: idle\r\n$ "),
            other => panic!("expected idle-pumped terminal output, got {other:?}"),
        }

        socket
            .send(Message::binary(
                b"qjs ../foreground-child.js\nserved foreground input\n".as_slice(),
            ))
            .unwrap();
        match socket.read().unwrap() {
            Message::Binary(bytes) => assert_eq!(
                bytes.as_ref(),
                b"qjs ../foreground-child.js\r\nserved foreground stdin served foreground input\r\n$ "
            ),
            other => panic!("expected foreground child terminal output, got {other:?}"),
        }

        socket
            .send(Message::binary(
                b"qjs ../terminal-child.js\nsetenv MODE served-mode\nqjs ../child.js from ws < inside.txt > child-out.txt 2> child-err.txt\nstatus\ncat child-out.txt\ncat child-err.txt\nexit\n".as_slice(),
            ))
            .unwrap();
        let mut transcript = Vec::new();
        let mut exit = None;
        while exit.is_none() {
            match socket.read().unwrap() {
                Message::Binary(bytes) => transcript.extend_from_slice(bytes.as_ref()),
                Message::Text(text) => exit = Some(text.to_string()),
                Message::Close(_) => break,
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).unwrap(),
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        let _ = socket.close(None);
        let (exit_code, stderr) = handle.join().unwrap();
        let stderr = String::from_utf8(stderr).unwrap();

        assert_eq!(exit_code, 0, "{stderr}");
        assert_eq!(
            transcript,
            b"qjs ../terminal-child.js\r\nserved terminal child stdout\r\nserved terminal child stderr\r\n$ setenv MODE served-mode\r\n$ qjs ../child.js from ws < inside.txt > child-out.txt 2> child-err.txt\r\nqjs exit 6\r\n$ status\r\nstatus 6\r\n$ cat child-out.txt\r\nserved child task 4\r\nserved child cwd .\r\nserved child argv child.js|from|ws\r\nserved child stdin inside app\r\nserved child mode served-mode\r\n$ cat child-err.txt\r\nserved child stderr from\r\n$ exit\r\nbye\r\n"
        );
        assert_eq!(exit.as_deref(), Some("{\"type\":\"exit\",\"code\":0}"));
    }

    #[test]
    fn qjs_shell_websocket_cwd_query_uses_wanix_paths() {
        assert_eq!(
            qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell"))
                .map(|cwd| cwd.to_string())
                .unwrap_or_else(|_| panic!("default qjs shell cwd should parse")),
            "."
        );
        assert_eq!(
            qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=%2Fapp%2Fsub"))
                .map(|cwd| cwd.to_string())
                .unwrap_or_else(|_| panic!("absolute qjs shell cwd should parse")),
            "app/sub"
        );
        assert_eq!(
            qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=%2F"))
                .map(|cwd| cwd.to_string())
                .unwrap_or_else(|_| panic!("root qjs shell cwd should parse")),
            "."
        );

        let response =
            qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=../escape")).unwrap_err();
        assert!(matches!(response.status, HttpStatus::BadRequest));
    }

    #[test]
    fn qjs_shell_websocket_resize_messages_parse_json_and_text() {
        assert_eq!(
            parse_terminal_resize_message(r#"{"type":"resize","columns":100,"rows":40}"#),
            Some((100, 40))
        );
        assert_eq!(
            parse_terminal_resize_message("resize 120 50"),
            Some((120, 50))
        );
        assert_eq!(parse_terminal_resize_message("resize 0 50"), None);
    }

    #[test]
    fn serve_once_exports_google_2_walkgetattr_on_well_known_path() {
        let root = temp_dir("wanix-cli-serve-export9p-google2");
        fs::write(root.join("hello.txt"), b"hello google2").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalkgetattr(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 13),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 5);
        socket.close(None).unwrap();
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RATTACH,
                P9_RWALKGETATTR,
                P9_RLOPEN,
                P9_RREAD
            ]
        );
        let walk = p9_decode_rwalkgetattr(&frames[2]).unwrap();
        assert_eq!(walk.valid, u64::MAX);
        assert_eq!(walk.qids.len(), 1);
        assert_eq!(walk.attr.size, 13);
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello google2");
    }

    #[test]
    fn serve_once_exports_9p_compatibility_probes_on_well_known_path() {
        let root = temp_dir("wanix-cli-serve-export9p-compat-probes");
        fs::write(root.join("target.txt"), b"target").unwrap();
        let root_check = root.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/.well-known/export9p"))
            .unwrap()
            .0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tauth(2, 9, "root", "", 0).unwrap(),
            p9_tattach(3, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(4, 1, 2, &["target.txt"]).unwrap(),
            p9_tmknod(5, 1, "tty0", 0o020620, 4, 0, 0).unwrap(),
            p9_tlink(6, 1, 2, "hard.txt").unwrap(),
            p9_txattrwalk(7, 2, 3, "user.foo").unwrap(),
            p9_txattrcreate(8, 2, "user.foo", 12, 0).unwrap(),
            p9_trename(9, 2, 1, "renamed.txt").unwrap(),
            p9_tgetattr(10, 2, u64::MAX),
            p9_tremove(11, 2),
            p9_tgetattr(12, 2, u64::MAX),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 12);
        socket.close(None).unwrap();
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RLERROR,
                P9_RATTACH,
                P9_RWALK,
                P9_RLERROR,
                P9_RLINK,
                P9_RLERROR,
                P9_RLERROR,
                P9_RRENAME,
                P9_RGETATTR,
                P9_RREMOVE,
                P9_RLERROR
            ]
        );
        assert_eq!(
            frames.iter().map(P9Frame::tag).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
        assert_eq!(p9_decode_rlerror(&frames[1]).unwrap().ecode, ENOSYS);
        assert_eq!(p9_decode_rlerror(&frames[4]).unwrap().ecode, EOPNOTSUPP);
        p9_decode_rlink(&frames[5]).unwrap();
        assert_eq!(p9_decode_rlerror(&frames[6]).unwrap().ecode, EOPNOTSUPP);
        assert_eq!(p9_decode_rlerror(&frames[7]).unwrap().ecode, EOPNOTSUPP);
        p9_decode_rrename(&frames[8]).unwrap();
        assert_eq!(p9_decode_rgetattr(&frames[9]).unwrap().size, 6);
        p9_decode_rremove(&frames[10]).unwrap();
        assert_eq!(p9_decode_rlerror(&frames[11]).unwrap().ecode, EBADF);
        assert!(!root_check.join("target.txt").exists());
        assert!(!root_check.join("renamed.txt").exists());
        assert_eq!(fs::read(root_check.join("hard.txt")).unwrap(), b"target");
    }

    #[test]
    fn serve_once_rejects_reserved_ethernet_websocket_path() {
        let root = temp_dir("wanix-cli-serve-ethernet");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/ethernet HTTP/1.1\r\n\
              Host: localhost\r\n\
              Connection: Upgrade\r\n\
              Upgrade: websocket\r\n\
              Sec-WebSocket-Version: 13\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
              \r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 501 Not Implemented\r\n"),
            "{response}"
        );
        assert!(
            response.contains("ethernet websocket bridge is not implemented"),
            "{response}"
        );
    }

    #[test]
    fn serve_http_keeps_well_known_routes_reserved() {
        let root = temp_dir("wanix-cli-serve-well-known-http");
        fs::create_dir(root.join(".well-known")).unwrap();
        fs::write(root.join(".well-known").join("export9p"), b"not static").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
            wanix_services: false,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/export9p HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{response}"
        );
        assert!(
            response.contains("websocket upgrade required"),
            "{response}"
        );
        assert!(!response.contains("not static"), "{response}");
    }

    #[test]
    fn serve_http_keeps_qjs_shell_route_reserved_when_services_enabled() {
        let root = temp_dir("wanix-cli-serve-qjs-shell-http");
        fs::create_dir(root.join(".well-known")).unwrap();
        fs::write(root.join(".well-known").join("qjs-shell"), b"not static").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
            wanix_services: true,
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let response = http_request(
            addr,
            b"GET /.well-known/qjs-shell HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let (exit_code, _stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{response}"
        );
        assert!(
            response.contains("websocket upgrade required"),
            "{response}"
        );
        assert!(!response.contains("not static"), "{response}");
    }

    fn p9_read_file(
        server: &mut wanix_9p::P9Server,
        root_fid: u32,
        fid: u32,
        tag_base: u16,
        path: &[&str],
    ) -> Vec<u8> {
        let response = server
            .handle_frame(&p9_twalk(tag_base, root_fid, fid, path).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), path.len());
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(tag_base + 1, fid, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server
            .handle_frame(&p9_tread(tag_base + 2, fid, 0, 4096))
            .unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        p9_decode_rread(&response).unwrap()
    }

    fn p9_write_file(
        server: &mut wanix_9p::P9Server,
        root_fid: u32,
        fid: u32,
        tag_base: u16,
        path: &[&str],
        bytes: &[u8],
    ) {
        let response = server
            .handle_frame(&p9_twalk(tag_base, root_fid, fid, path).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), path.len());
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(tag_base + 1, fid, P9_O_WRONLY))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let response = server
            .handle_frame(&p9_twrite(tag_base + 2, fid, 0, bytes).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), bytes.len() as u32);
    }

    fn read_binary_frames<S: Read + Write>(
        socket: &mut tungstenite::WebSocket<S>,
        count: usize,
    ) -> Vec<P9Frame> {
        let mut frames = Vec::new();
        while frames.len() < count {
            if let Message::Binary(bytes) = socket.read().unwrap() {
                frames.push(P9Frame::decode(&bytes).unwrap());
            }
        }
        frames
    }

    fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut stream = Vec::new();
        for frame in frames {
            stream.extend_from_slice(&frame.encode().unwrap());
        }
        stream
    }

    fn frame_types(frames: &[P9Frame]) -> Vec<u8> {
        frames.iter().map(P9Frame::message_type).collect()
    }

    fn http_request(addr: std::net::SocketAddr, request: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(request).unwrap();
        let mut response = Vec::new();
        match stream.read_to_end(&mut response) {
            Ok(_) => {}
            Err(error)
                if error.kind() == io::ErrorKind::ConnectionReset && !response.is_empty() => {}
            Err(error) => panic!("failed to read HTTP response: {error}"),
        }
        response
    }

    fn http_response_parts(response: &[u8]) -> (String, &[u8]) {
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        (
            String::from_utf8_lossy(&response[..header_end]).into_owned(),
            &response[header_end..],
        )
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_boot_init(root: &Path) {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let init = bin.join("init");
        fs::write(&init, b"init").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(init, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
