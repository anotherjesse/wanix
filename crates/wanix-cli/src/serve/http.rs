use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
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

pub(super) struct StaticResponse {
    pub(super) status: HttpStatus,
    pub(super) content_type: &'static str,
    pub(super) body: Vec<u8>,
}

impl StaticResponse {
    pub(super) fn plain(status: HttpStatus, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    pub(super) fn encode(&self) -> Vec<u8> {
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
pub(super) enum HttpStatus {
    Ok,
    BadRequest,
    Conflict,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotImplemented,
}

impl HttpStatus {
    pub(super) fn status_line(self) -> &'static str {
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

    pub(super) fn reason(self) -> &'static str {
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
