use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tungstenite::accept;
use wanix_fs::{FileSystem, LocalFs};

use crate::p9_ws::{P9WsConnectionError, serve_websocket_connection};
use crate::{CliError, write_process_output};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServeCommand {
    root_path: PathBuf,
    addr: String,
    once: bool,
}

pub(super) fn parse_serve_command(args: &[OsString]) -> Result<ServeCommand, CliError> {
    let mut root_path = None;
    let mut addr = None;
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
        } else if args[i] == "--addr" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("serve --addr expects HOST:PORT"))?;
            if addr.is_some() {
                return Err(CliError::usage("serve accepts only one --addr"));
            }
            addr = Some(value.to_string_lossy().into_owned());
            i += 1;
        } else if args[i] == "--once" {
            if once {
                return Err(CliError::usage("serve accepts only one --once"));
            }
            once = true;
            i += 1;
        } else {
            return Err(CliError::usage(format!(
                "unexpected serve argument: {}",
                args[i].to_string_lossy()
            )));
        }
    }

    let root_path = root_path.ok_or_else(|| CliError::usage("serve requires --root DIR"))?;
    let addr = addr.ok_or_else(|| CliError::usage("serve requires --addr HOST:PORT"))?;
    Ok(ServeCommand {
        root_path,
        addr,
        once,
    })
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
    let roots = ServeRoots::new(&command.root_path)?;

    write_process_output(
        process_stderr,
        "stderr",
        format!("wanix-rust serve: listening on http://{local_addr}/\n").as_bytes(),
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

#[derive(Clone)]
struct ServeRoots {
    static_root: PathBuf,
    p9_root: Arc<dyn FileSystem>,
}

impl ServeRoots {
    fn new(root_path: &Path) -> Result<Self, CliError> {
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
        let socket = accept(stream).map_err(|error| {
            ServeConnectionError::WebSocket(P9WsConnectionError::Handshake(error.to_string()))
        })?;
        return serve_websocket_connection(Arc::clone(&roots.p9_root), socket)
            .map_err(ServeConnectionError::WebSocket);
    }

    serve_http_connection(&roots.static_root, stream)
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
    static_root: &Path,
    mut stream: TcpStream,
) -> Result<(), ServeConnectionError> {
    let request = read_http_request(&mut stream)?;
    let response = match parse_http_request(&request) {
        Ok(path) => read_static_response(static_root, &path),
        Err(status) => StaticResponse::plain(status, status.reason()),
    };
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
}

impl HttpStatus {
    fn status_line(self) -> &'static str {
        match self {
            Self::Ok => "200 OK",
            Self::BadRequest => "400 Bad Request",
            Self::Forbidden => "403 Forbidden",
            Self::NotFound => "404 Not Found",
            Self::MethodNotAllowed => "405 Method Not Allowed",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::BadRequest => "bad request",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not found",
            Self::MethodNotAllowed => "method not allowed",
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tungstenite::{Message, connect};
    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L,
        P9Frame, p9_decode_rgetattr, p9_decode_rread, p9_tattach, p9_tgetattr, p9_tlopen, p9_tread,
        p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn parse_serve_requires_root_and_addr() {
        let error =
            parse_serve_command(&[OsString::from("--root"), OsString::from(".")]).unwrap_err();
        assert!(error.to_string().contains("requires --addr HOST:PORT"));

        let command = parse_serve_command(&[
            OsString::from("--root"),
            OsString::from("."),
            OsString::from("--addr"),
            OsString::from("127.0.0.1:0"),
            OsString::from("--once"),
        ])
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("."));
        assert_eq!(command.addr, "127.0.0.1:0");
        assert!(command.once);
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
    fn serve_once_exports_9p_over_binary_websocket() {
        let root = temp_dir("wanix-cli-serve-ws");
        fs::write(root.join("hello.txt"), b"hello serve").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = ServeCommand {
            root_path: root,
            addr: addr.to_string(),
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
