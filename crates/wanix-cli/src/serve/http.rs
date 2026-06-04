use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::{ServeRoots, connection::ServeConnectionError};

mod response;
mod routes;

pub(super) use response::{HttpStatus, StaticResponse};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;

pub(super) fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(super) fn peek_request_target(header_bytes: &[u8]) -> Option<&str> {
    request_target(header_bytes)
}

pub(super) fn peek_request_headers(stream: &TcpStream) -> io::Result<Vec<u8>> {
    let mut buffer = [0; MAX_HTTP_HEADER_BYTES];
    loop {
        let len = stream.peek(&mut buffer)?;
        if len == 0 || header_end(&buffer[..len]).is_some() || len == buffer.len() {
            return Ok(buffer[..len].to_vec());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn is_websocket_upgrade(header_bytes: &[u8]) -> bool {
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

pub(super) fn parse_http_request(bytes: &[u8]) -> Result<PathBuf, HttpStatus> {
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

pub(super) fn percent_decode(segment: &str) -> Result<String, HttpStatus> {
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

pub(super) fn read_static_response(static_root: &Path, relative_path: &Path) -> StaticResponse {
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

pub(super) fn serve_http_connection(
    roots: &ServeRoots,
    mut stream: TcpStream,
    peer_addr: SocketAddr,
) -> Result<(), ServeConnectionError> {
    let request = read_http_request(&mut stream)?;
    let response = match parse_http_request(&request) {
        Ok(path) => routes::http_route_response(roots, &path, &request, peer_addr)
            .unwrap_or_else(|| read_static_response(&roots.static_root, &path)),
        Err(status) => StaticResponse::plain(status, status.reason()),
    };
    write_static_response(stream, response)
}

pub(super) fn write_static_response(
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

pub(super) fn websocket_rejection_response(raw_path: Option<&str>) -> Option<StaticResponse> {
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

fn request_target(header_bytes: &[u8]) -> Option<&str> {
    let header_end = header_end(header_bytes)?;
    let header = std::str::from_utf8(&header_bytes[..header_end]).ok()?;
    let request_line = header.lines().next()?;
    let mut parts = request_line.split_whitespace();
    parts.next()?;
    parts.next()
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
