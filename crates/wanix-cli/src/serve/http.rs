use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use wanix_sites::Host;

use super::{ServeRoots, connection::ServeConnectionError};

mod agent;
pub(super) mod app;
mod request;
mod response;
mod routes;
mod static_serve;

pub(super) use request::{
    header_end, parse_http_request, peek_request_target, percent_decode, request_target,
    websocket_rejection_response,
};
pub(super) use response::{HttpStatus, StaticResponse};
pub(super) use static_serve::{
    path_to_url_relative, read_disk_static_response, read_static_response,
};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const HTTP_READ_BUFFER_BYTES: usize = 1024;

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

pub(super) fn serve_http_connection(
    roots: &ServeRoots,
    mut stream: TcpStream,
    peer_addr: SocketAddr,
) -> Result<(), ServeConnectionError> {
    let request = read_http_request(&mut stream)?;
    let response = http_response(roots, &request, peer_addr);
    write_static_response(stream, response)
}

pub(super) fn http_response(
    roots: &ServeRoots,
    request: &[u8],
    peer_addr: SocketAddr,
) -> StaticResponse {
    if request_method(request).as_deref() == Some("POST") {
        return match request_line_path(request).as_deref() {
            Some("/agent") => agent::agent_endpoint(roots, request_body(request), peer_addr),
            _ => StaticResponse::plain(HttpStatus::NotFound, "not found"),
        };
    }
    match parse_http_request(request) {
        Ok(path) => {
            let url_relative = path_to_url_relative(&path);
            routes::http_route_response(roots, &path, request, peer_addr)
                .or_else(|| site_gateway_response(roots, request, &url_relative))
                .unwrap_or_else(|| read_static_response(roots.site_root.as_ref(), &url_relative))
        }
        Err(status) => StaticResponse::plain(status, status.reason()),
    }
}

/// Routes a request to a host-bound site (the `*.localhost` gateway).
///
/// Reads the `Host` header, strips its port, and looks the host up in `#sites`.
/// A bound, non-bare host serves through the Phase 0 FS-backed handler against
/// that site's filesystem; a bare `localhost`, an IP literal, or an unbound
/// host returns `None` so the caller falls through to the `--root` behavior.
/// Only active when `--wanix-services` bound the `#sites` device.
fn site_gateway_response(
    roots: &ServeRoots,
    request: &[u8],
    url_relative: &str,
) -> Option<StaticResponse> {
    if !roots.wanix_services {
        return None;
    }
    let host = Host::parse(request_host_header(request)?)?;
    let site_fs = roots.sites.resolve(&host)?;
    Some(read_static_response(site_fs.as_ref(), url_relative))
}

/// Returns the raw `Host` header value, if present.
fn request_host_header(request: &[u8]) -> Option<&str> {
    request_header_str(request)?
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("host")
                .then(|| value.trim())
        })
}

fn request_header_str(request: &[u8]) -> Option<&str> {
    std::str::from_utf8(&request[..header_end(request)?]).ok()
}

fn request_method(request: &[u8]) -> Option<String> {
    request_header_str(request)?
        .lines()
        .next()?
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

fn request_line_path(request: &[u8]) -> Option<String> {
    let mut parts = request_header_str(request)?
        .lines()
        .next()?
        .split_whitespace();
    parts.next()?;
    parts
        .next()
        .map(|target| target.split('?').next().unwrap_or(target).to_owned())
}

fn content_length(request: &[u8]) -> Option<usize> {
    request_header_str(request)?
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
}

fn request_body(request: &[u8]) -> &[u8] {
    match header_end(request) {
        Some(end) => &request[(end + 4).min(request.len())..],
        None => &[],
    }
}

pub(super) fn write_static_response(
    mut stream: TcpStream,
    response: StaticResponse,
) -> Result<(), ServeConnectionError> {
    stream
        .write_all(&response.encode())
        .map_err(ServeConnectionError::Io)?;
    stream.flush().map_err(ServeConnectionError::Io)?;
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, ServeConnectionError> {
    let mut request = Vec::new();
    let mut buffer = [0; HTTP_READ_BUFFER_BYTES];
    while request.len() < MAX_HTTP_HEADER_BYTES {
        let len = stream.read(&mut buffer).map_err(ServeConnectionError::Io)?;
        if len == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..len]);
        if header_end(&request).is_some() {
            break;
        }
    }
    let Some(end) = header_end(&request) else {
        return Err(ServeConnectionError::Http(
            "HTTP request header was incomplete or too large".to_owned(),
        ));
    };
    if let Some(length) = content_length(&request) {
        let target = (end + 4).saturating_add(length.min(MAX_HTTP_BODY_BYTES));
        while request.len() < target {
            let len = stream.read(&mut buffer).map_err(ServeConnectionError::Io)?;
            if len == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..len]);
        }
    }
    Ok(request)
}
