use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::thread;
use std::time::Duration;

use super::{ServeRoots, connection::ServeConnectionError};

mod agent;
pub(super) mod app;
mod request;
mod response;
mod routes;

pub(super) use request::{
    header_end, parse_http_request, peek_request_target, percent_decode, request_target,
    websocket_rejection_response,
};
pub(super) use response::{HttpStatus, StaticResponse};

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

/// Serves the static path through a [`wanix_fs::FileSystem`] (Phase 0).
///
/// All static-file I/O goes through the filesystem trait, not `std::fs`:
/// directory-index resolution, path confinement, and the byte read are owned by
/// [`wanix_site_fs::read_site_file`]. The `--root DIR` path passes a `LocalFs`
/// here, so its behavior is unchanged; other phases serve other filesystems.
pub(super) fn read_static_response(
    fs: &dyn wanix_fs::FileSystem,
    url_relative: &str,
) -> StaticResponse {
    match wanix_site_fs::read_site_file(fs, url_relative) {
        wanix_site_fs::SiteFile::Found { bytes, extension } => StaticResponse {
            status: HttpStatus::Ok,
            content_type: content_type_for_extension(extension.as_deref()),
            headers: Vec::new(),
            body: bytes,
        },
        wanix_site_fs::SiteFile::NotFound => {
            StaticResponse::plain(HttpStatus::NotFound, "not found")
        }
        wanix_site_fs::SiteFile::Forbidden => {
            StaticResponse::plain(HttpStatus::Forbidden, "forbidden")
        }
    }
}

/// Serves a static file directly from a host directory via `std::fs`.
///
/// Retained for the workbench asset root, which is a real host directory
/// outside the served root; keeping it on the proven disk path avoids
/// constructing a `LocalFs` per request for those assets.
pub(super) fn read_disk_static_response(
    static_root: &Path,
    relative_path: &Path,
) -> StaticResponse {
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
            headers: Vec::new(),
            body,
        },
        Err(_) => StaticResponse::plain(HttpStatus::NotFound, "not found"),
    }
}

/// Joins the normal components of a request path into a `/`-separated URL
/// relative string for the FS-backed static handler. `parse_http_request`
/// already rejected traversal, so every component is a plain segment.
fn path_to_url_relative(relative_path: &Path) -> String {
    relative_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
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

fn http_response(roots: &ServeRoots, request: &[u8], peer_addr: SocketAddr) -> StaticResponse {
    if request_method(request).as_deref() == Some("POST") {
        return match request_line_path(request).as_deref() {
            Some("/agent") => agent::agent_endpoint(roots, request_body(request), peer_addr),
            _ => StaticResponse::plain(HttpStatus::NotFound, "not found"),
        };
    }
    match parse_http_request(request) {
        Ok(path) => {
            routes::http_route_response(roots, &path, request, peer_addr).unwrap_or_else(|| {
                read_static_response(roots.site_root.as_ref(), &path_to_url_relative(&path))
            })
        }
        Err(status) => StaticResponse::plain(status, status.reason()),
    }
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

fn content_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|extension| extension.to_str());
    content_type_for_extension(extension)
}

/// Maps a (lowercased) file extension to a static content type. Keys on the
/// extension only, so it works identically for a host `Path` and a virtual
/// filesystem path.
fn content_type_for_extension(extension: Option<&str>) -> &'static str {
    const TYPES: &[(&str, &str)] = &[
        ("html", "text/html; charset=utf-8"),
        ("js", "text/javascript; charset=utf-8"),
        ("mjs", "text/javascript; charset=utf-8"),
        ("css", "text/css; charset=utf-8"),
        ("json", "application/json"),
        ("wasm", "application/wasm"),
        ("txt", "text/plain; charset=utf-8"),
    ];
    TYPES
        .iter()
        .find_map(|(known_extension, content_type)| {
            extension
                .filter(|extension| extension.eq_ignore_ascii_case(known_extension))
                .map(|_| *content_type)
        })
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
mod tests {
    use wanix_fs::MemFs;

    use super::{path_to_url_relative, read_static_response};

    /// Proves the static HTTP path serves through the `FileSystem` trait with
    /// no files on disk: an in-memory `MemFs` answers GETs with correct bytes,
    /// content-types, and directory-index resolution.
    #[test]
    fn read_static_response_serves_in_memory_fs_with_no_disk() {
        let fs = MemFs::new();
        fs.write_file("index.html", b"<h1>home</h1>").unwrap();
        fs.write_file("concepts/foo/index.html", b"<h1>foo</h1>")
            .unwrap();
        fs.write_file("style.css", b"body{color:red}").unwrap();

        // Root resolves to index.html via directory-index.
        let root = read_static_response(&fs, &path_to_url_relative(std::path::Path::new("")));
        assert_eq!(root.status.status_line(), "200 OK");
        assert_eq!(root.content_type, "text/html; charset=utf-8");
        assert_eq!(root.body, b"<h1>home</h1>");

        // A nested directory resolves to its index.html.
        let nested = read_static_response(&fs, "concepts/foo");
        assert_eq!(nested.status.status_line(), "200 OK");
        assert_eq!(nested.content_type, "text/html; charset=utf-8");
        assert_eq!(nested.body, b"<h1>foo</h1>");

        // A CSS file keeps its content type and exact bytes.
        let css = read_static_response(&fs, "style.css");
        assert_eq!(css.status.status_line(), "200 OK");
        assert_eq!(css.content_type, "text/css; charset=utf-8");
        assert_eq!(css.body, b"body{color:red}");

        // A miss is a 404, not a server error.
        let missing = read_static_response(&fs, "nope.html");
        assert_eq!(missing.status.status_line(), "404 Not Found");
    }
}
