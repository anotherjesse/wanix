use std::path::PathBuf;

use super::{HttpStatus, StaticResponse};

const PERCENT_ESCAPE_BYTES: usize = 3;
const HEX_HIGH_NIBBLE_SHIFT: u8 = 4;
const HEX_ALPHA_OFFSET: u8 = 10;

pub(in crate::serve) fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(in crate::serve) fn peek_request_target(header_bytes: &[u8]) -> Option<&str> {
    request_target(header_bytes)
}

pub(in crate::serve) fn parse_http_request(bytes: &[u8]) -> Result<PathBuf, HttpStatus> {
    let header = request_header(bytes)?;
    let target = request_line_target(header)?;
    let mut path = safe_relative_path(target)?;
    if path.as_os_str().is_empty() {
        path.push("index.html");
    }
    Ok(path)
}

/// The request path for the WebDoor gateway: any method (the gateway maps
/// verbs itself), and `/` stays the empty path (the origin root — a directory
/// request, not an implicit `index.html`).
pub(in crate::serve) fn gateway_request_path(bytes: &[u8]) -> Result<PathBuf, HttpStatus> {
    let header = request_header(bytes)?;
    let request_line = first_header_line(header)?;
    let (_method, raw_path, version) = request_line_parts(request_line)?;
    validate_http_version(version)?;
    safe_relative_path(raw_path)
}

fn request_header(bytes: &[u8]) -> Result<&str, HttpStatus> {
    let end = header_end(bytes).ok_or(HttpStatus::BadRequest)?;
    std::str::from_utf8(&bytes[..end]).map_err(|_| HttpStatus::BadRequest)
}

fn request_line_target(header: &str) -> Result<&str, HttpStatus> {
    let request_line = first_header_line(header)?;
    let (method, raw_path, version) = request_line_parts(request_line)?;
    validate_http_version(version)?;
    require_get_method(method)?;
    Ok(raw_path)
}

fn first_header_line(header: &str) -> Result<&str, HttpStatus> {
    header.lines().next().ok_or(HttpStatus::BadRequest)
}

fn request_line_parts(request_line: &str) -> Result<(&str, &str, &str), HttpStatus> {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(HttpStatus::BadRequest)?;
    let raw_path = parts.next().ok_or(HttpStatus::BadRequest)?;
    let version = parts.next().ok_or(HttpStatus::BadRequest)?;
    reject_extra_request_line_parts(parts.next())?;
    Ok((method, raw_path, version))
}

fn reject_extra_request_line_parts(extra_part: Option<&str>) -> Result<(), HttpStatus> {
    if extra_part.is_some() {
        return Err(HttpStatus::BadRequest);
    }
    Ok(())
}

fn validate_http_version(version: &str) -> Result<(), HttpStatus> {
    if version.starts_with("HTTP/1.") {
        return Ok(());
    }
    Err(HttpStatus::BadRequest)
}

fn require_get_method(method: &str) -> Result<(), HttpStatus> {
    if method == "GET" {
        return Ok(());
    }
    Err(HttpStatus::MethodNotAllowed)
}

fn safe_relative_path(raw_path: &str) -> Result<PathBuf, HttpStatus> {
    let path_without_query = raw_path.split_once('?').map_or(raw_path, |(path, _)| path);
    if !path_without_query.starts_with('/') {
        return Err(HttpStatus::BadRequest);
    }

    let mut path = PathBuf::new();
    for raw_segment in path_without_query.split('/') {
        push_safe_segment(&mut path, raw_segment)?;
    }
    Ok(path)
}

fn push_safe_segment(path: &mut PathBuf, raw_segment: &str) -> Result<(), HttpStatus> {
    if raw_segment.is_empty() {
        return Ok(());
    }
    let segment = percent_decode(raw_segment)?;
    if is_forbidden_path_segment(&segment) {
        return Err(HttpStatus::Forbidden);
    }
    path.push(segment);
    Ok(())
}

fn is_forbidden_path_segment(segment: &str) -> bool {
    segment == "." || segment == ".." || segment.contains('/') || segment.contains('\\')
}

pub(in crate::serve) fn percent_decode(segment: &str) -> Result<String, HttpStatus> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        i = push_percent_decoded_byte(bytes, i, &mut decoded)?;
    }
    String::from_utf8(decoded).map_err(|_| HttpStatus::BadRequest)
}

fn push_percent_decoded_byte(
    bytes: &[u8],
    index: usize,
    decoded: &mut Vec<u8>,
) -> Result<usize, HttpStatus> {
    match bytes[index] {
        b'%' => push_percent_escape(bytes, index, decoded),
        byte => push_raw_byte(byte, index, decoded),
    }
}

fn push_raw_byte(byte: u8, index: usize, decoded: &mut Vec<u8>) -> Result<usize, HttpStatus> {
    decoded.push(byte);
    Ok(index + 1)
}

fn push_percent_escape(
    bytes: &[u8],
    index: usize,
    decoded: &mut Vec<u8>,
) -> Result<usize, HttpStatus> {
    let high = *bytes.get(index + 1).ok_or(HttpStatus::BadRequest)?;
    let low = *bytes.get(index + 2).ok_or(HttpStatus::BadRequest)?;
    decoded.push((hex_value(high)? << HEX_HIGH_NIBBLE_SHIFT) | hex_value(low)?);
    Ok(index + PERCENT_ESCAPE_BYTES)
}

fn hex_value(byte: u8) -> Result<u8, HttpStatus> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + HEX_ALPHA_OFFSET),
        b'A'..=b'F' => Ok(byte - b'A' + HEX_ALPHA_OFFSET),
        _ => Err(HttpStatus::BadRequest),
    }
}

pub(in crate::serve) fn websocket_rejection_response(
    raw_path: Option<&str>,
) -> Option<StaticResponse> {
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

pub(in crate::serve) fn request_target(header_bytes: &[u8]) -> Option<&str> {
    let header_end = header_end(header_bytes)?;
    let header = std::str::from_utf8(&header_bytes[..header_end]).ok()?;
    let request_line = header.lines().next()?;
    let mut parts = request_line.split_whitespace();
    parts.next()?;
    parts.next()
}
