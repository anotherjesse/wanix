use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tungstenite::accept;
use wanix_fs::{FileSystem, LocalFs};

use crate::p9_ws::{P9WsConnectionError, serve_websocket_connection};
use crate::{CliError, write_process_output};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7654";
const DIRECT_V86_BUNDLE: &str = "direct-v86";
const DIRECT_V86_DEFAULT_CMDLINE: &str = "console=hvc0 init=/bin/init rw root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L,aname=,cache=none,msize=131072 loglevel=3";
const DIRECT_V86_MEMORY_SIZE: u32 = 1024 * 1024 * 1024;
const DIRECT_V86_VGA_MEMORY_SIZE: u32 = 8 * 1024 * 1024;
const DIRECT_V86_MODULE_PATH: &str = "/v86/lib/libv86.mjs";
const DIRECT_V86_MOD_REEXPORT_PATH: &str = "/v86/lib/mod.js";
const DIRECT_V86_OFFSCREEN_PATH: &str = "/v86/lib/offscreen.js";
const DIRECT_V86_WASM_PATH: &str = "/v86/bundle/v86.wasm";
const DIRECT_V86_BIOS_PATH: &str = "/v86/bundle/seabios.bin";
const DIRECT_V86_VGA_BIOS_PATH: &str = "/v86/bundle/vgabios.bin";

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
    once: bool,
}

pub(super) fn parse_serve_command(args: &[OsString]) -> Result<ServeCommand, CliError> {
    let mut root_path = None;
    let mut addr = None;
    let mut bundle = None;
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
    let local_addr = listener
        .local_addr()
        .map_err(|error| CliError::new(format!("failed to inspect serve address: {error}"), 1))?;
    let roots = ServeRoots::new(&command.root_path, local_addr, command.bundle.clone())?;

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

    loop {
        let exit_code = serve_one_connection(&listener, &roots, process_stderr)?;
        if exit_code != 0 {
            write_process_output(
                process_stderr,
                "stderr",
                b"wanix-rust serve: continuing after connection error\n",
            )?;
        }
    }
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
}

impl ServeRoots {
    fn new(
        root_path: &Path,
        local_addr: SocketAddr,
        bundle: Option<String>,
    ) -> Result<Self, CliError> {
        let static_root = fs::canonicalize(root_path).map_err(|error| {
            CliError::new(
                format!("failed to open serve root {}: {error}", root_path.display()),
                1,
            )
        })?;
        let p9_root = LocalFs::new(root_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to open serve 9P root {}: {error}",
                    root_path.display()
                ),
                1,
            )
        })?;
        Ok(Self {
            static_root,
            p9_root: Arc::new(p9_root),
            local_addr,
            bundle,
        })
    }
}

fn serve_one_connection(
    listener: &TcpListener,
    roots: &ServeRoots,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("serve accept failed: {error}"), 1))?;
    match serve_connection(roots, stream) {
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

fn serve_connection(roots: &ServeRoots, stream: TcpStream) -> Result<(), ServeConnectionError> {
    let request = peek_request_headers(&stream)?;
    if is_websocket_upgrade(&request) {
        if let Some(response) = websocket_rejection_response(peek_request_target(&request)) {
            return write_static_response(stream, response);
        }
        let socket = accept(stream).map_err(|error| {
            ServeConnectionError::WebSocket(P9WsConnectionError::Handshake(error.to_string()))
        })?;
        return serve_websocket_connection(Arc::clone(&roots.p9_root), socket)
            .map_err(ServeConnectionError::WebSocket);
    }

    serve_http_connection(roots, stream)
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
) -> Result<(), ServeConnectionError> {
    let request = read_http_request(&mut stream)?;
    let response = match parse_http_request(&request) {
        Ok(path) => well_known_response(roots, &path, &request)
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
                Some(serve_discovery_response(roots, request))
            }
        }
        Some(Component::Normal(component)) if component == "export9p" && !has_extra_components => {
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
    if roots.bundle.as_deref() != Some(DIRECT_V86_BUNDLE)
        || relative_path != Path::new("index.html")
    {
        return None;
    }
    let target = http_request_target(request)?;
    let (_, query) = target.split_once('?')?;
    if query_param(query, "bundle") != Some(DIRECT_V86_BUNDLE) {
        return None;
    }
    Some(direct_v86_bundle_response())
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
    #serial { min-height: 180px; resize: vertical; }
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
      <input id="kernel" value="/bzImage">
      <label for="initrd">initrd URL</label>
      <input id="initrd" value="">
      <label for="cmdline">kernel command line</label>
      <textarea id="cmdline" spellcheck="false"></textarea>
      <button id="start" disabled>Start VM</button>
      <pre id="config"></pre>
    </aside>
    <div id="screen"><canvas></canvas><div></div></div>
    <textarea id="serial" spellcheck="false"></textarea>
  </main>
  <script type="module">
"##);
    html.push_str("    const DEFAULT_CMDLINE = ");
    html.push_str(&json_string(DIRECT_V86_DEFAULT_CMDLINE));
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
    const kernel = document.querySelector("#kernel");
    const initrd = document.querySelector("#initrd");
    const cmdline = document.querySelector("#cmdline");
    const params = new URLSearchParams(location.search);
    if (params.get("kernel")) kernel.value = params.get("kernel");
    if (params.get("bzimage")) kernel.value = params.get("bzimage");
    if (params.get("initrd")) initrd.value = params.get("initrd");

    const discovery = await fetch("/.well-known/wanix.json", { cache: "no-store" }).then(response => response.json());
    const v86Assets = discovery.v86?.assets || {};
    const { V86 } = await import(v86Assets.module || "/v86/lib/libv86.mjs");
    const proxyUrl = discovery.routes.p9.websocket;
    cmdline.value = discovery.v86?.defaultCmdline || DEFAULT_CMDLINE;
    if (params.get("cmdline")) cmdline.value = params.get("cmdline");
    if (params.get("append")) cmdline.value = [cmdline.value, params.get("append")].filter(Boolean).join(" ");
    status.textContent = "9P proxy: " + proxyUrl;

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

    kernel.addEventListener("input", refreshConfig);
    initrd.addEventListener("input", refreshConfig);
    cmdline.addEventListener("input", refreshConfig);
    refreshConfig();
    start.disabled = false;
    start.addEventListener("click", () => {
      start.disabled = true;
      new V86(buildConfig());
    });
  </script>
</body>
</html>
"##);
    html
}

fn serve_discovery_response(roots: &ServeRoots, request: &[u8]) -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "application/json",
        body: serve_discovery_json(roots.local_addr, roots.bundle.as_deref(), request).into_bytes(),
    }
}

fn serve_discovery_json(local_addr: SocketAddr, bundle: Option<&str>, request: &[u8]) -> String {
    let host = request_host(request).unwrap_or_else(|| display_host(local_addr));
    let p9_url = format!("ws://{host}/.well-known/export9p");
    let ethernet_url = format!("ws://{host}/.well-known/ethernet");
    let bundle = bundle.map(json_string).unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"version\":1,\
         \"runtime\":\"wanix-rust\",\
         \"routes\":{{\
         \"p9\":{{\"websocket\":{},\"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\"}},\
         \"ethernet\":{{\"websocket\":{},\"status\":\"not-implemented\"}}\
         }},\
         \"v86\":{{\"assets\":{{\"module\":{},\"mod\":{},\"offscreen\":{},\"wasm\":{},\"bios\":{},\"vgaBios\":{}}},\
         \"defaultCmdline\":{},\"memorySize\":{},\"vgaMemorySize\":{},\"virtioConsole\":true}},\
         \"bundle\":{}}}",
        json_string(&p9_url),
        json_string(&ethernet_url),
        json_string(DIRECT_V86_MODULE_PATH),
        json_string(DIRECT_V86_MOD_REEXPORT_PATH),
        json_string(DIRECT_V86_OFFSCREEN_PATH),
        json_string(DIRECT_V86_WASM_PATH),
        json_string(DIRECT_V86_BIOS_PATH),
        json_string(DIRECT_V86_VGA_BIOS_PATH),
        json_string(DIRECT_V86_DEFAULT_CMDLINE),
        DIRECT_V86_MEMORY_SIZE,
        DIRECT_V86_VGA_MEMORY_SIZE,
        bundle
    )
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

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
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
}

impl fmt::Display for ServeConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O failed: {error}"),
            Self::Http(error) => f.write_str(error),
            Self::WebSocket(error) => write!(f, "websocket 9P failed: {error}"),
        }
    }
}

impl Error for ServeConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::WebSocket(error) => Some(error),
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use tungstenite::{Message, connect};
    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLERROR, P9_RLOPEN, P9_RREAD, P9_RREMOVE, P9_RRENAME,
        P9_RSETATTR, P9_RVERSION, P9_RWALK, P9_SETATTR_GID, P9_SETATTR_UID, P9_VERSION_9P2000_L,
        P9Frame, P9SetAttr, p9_decode_rgetattr, p9_decode_rlerror, p9_decode_rread,
        p9_decode_rremove, p9_decode_rrename, p9_tattach, p9_tauth, p9_tgetattr, p9_tlink,
        p9_tlopen, p9_tmknod, p9_tread, p9_tremove, p9_trename, p9_tsetattr, p9_tversion, p9_twalk,
        p9_txattrcreate, p9_txattrwalk,
    };

    use super::*;

    const EBADF: u32 = 9;
    const ENOSYS: u32 = 38;
    const EOPNOTSUPP: u32 = 95;

    #[test]
    fn parse_serve_uses_go_like_defaults_and_options() {
        let default_command = parse_serve_command(&[]).unwrap();
        assert_eq!(default_command.root_path, PathBuf::from("."));
        assert_eq!(default_command.addr, DEFAULT_SERVE_ADDR);
        assert_eq!(default_command.bundle, None);
        assert!(!default_command.once);

        let command = parse_serve_command(&[
            OsString::from("examples"),
            OsString::from("--listen"),
            OsString::from(":7654"),
            OsString::from("--bundle"),
            OsString::from("vm-workbench"),
            OsString::from("--once"),
        ])
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("examples"));
        assert_eq!(command.addr, "0.0.0.0:7654");
        assert_eq!(command.bundle, Some("vm-workbench".to_owned()));
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
    fn serve_once_reports_bundle_url_when_configured() {
        let root = temp_dir("wanix-cli-serve-bundle");
        fs::write(root.join("index.html"), b"wanix serve bundle").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
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
            response.contains(
                "const { V86 } = await import(v86Assets.module || \"/v86/lib/libv86.mjs\")"
            ),
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
            response.contains("<textarea id=\"serial\" spellcheck=\"false\"></textarea>"),
            "{response}"
        );
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
        fs::write(root.join("index.html"), b"wanix serve discovery").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: Some("vm-workbench".to_owned()),
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
                 \"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\"}"
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
            response.contains("\"defaultCmdline\":\"console=hvc0 init=/bin/init rw root=host9p"),
            "{response}"
        );
        assert!(
            response.contains(
                "\"memorySize\":1073741824,\"vgaMemorySize\":8388608,\"virtioConsole\":true"
            ),
            "{response}"
        );
    }

    #[test]
    fn serve_discovery_falls_back_to_listener_address_without_host_header() {
        let local_addr = "0.0.0.0:7654".parse().unwrap();
        let body = serve_discovery_json(
            local_addr,
            Some("quote\"bundle"),
            b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
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

        let ipv6_addr = "[::1]:7654".parse().unwrap();
        let ipv6_body = serve_discovery_json(
            ipv6_addr,
            None,
            b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
        );
        assert!(
            ipv6_body.contains("\"websocket\":\"ws://[::1]:7654/.well-known/export9p\""),
            "{ipv6_body}"
        );
    }

    #[test]
    fn serve_discovery_ignores_unsafe_host_header() {
        let local_addr = "127.0.0.1:7654".parse().unwrap();
        let body = serve_discovery_json(
            local_addr,
            None,
            b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654/escape\r\n\r\n",
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
    fn serve_once_exports_9p_on_well_known_export_path() {
        let root = temp_dir("wanix-cli-serve-export9p");
        fs::write(root.join("hello.txt"), b"hello export").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
            bundle: None,
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
                P9_RLERROR,
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
        assert_eq!(p9_decode_rlerror(&frames[5]).unwrap().ecode, EOPNOTSUPP);
        assert_eq!(p9_decode_rlerror(&frames[6]).unwrap().ecode, EOPNOTSUPP);
        assert_eq!(p9_decode_rlerror(&frames[7]).unwrap().ecode, EOPNOTSUPP);
        p9_decode_rrename(&frames[8]).unwrap();
        assert_eq!(p9_decode_rgetattr(&frames[9]).unwrap().size, 6);
        p9_decode_rremove(&frames[10]).unwrap();
        assert_eq!(p9_decode_rlerror(&frames[11]).unwrap().ecode, EBADF);
        assert!(!root_check.join("target.txt").exists());
        assert!(!root_check.join("renamed.txt").exists());
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

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
