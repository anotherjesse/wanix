//! The WebDoor verb→filesystem mapping for one bound origin.
//!
//! - `GET file` → 200 with the bytes. A sized file is sent whole
//!   (`Content-Length`, content-type by extension); a zero-length file is
//!   treated as a device file and **streamed** as chunked transfer (never-EOF
//!   reads hold the connection open), or as SSE `data:` events when the
//!   request `Accept`s `text/event-stream`.
//! - `POST`/`PUT file` → write the body (`PUT` truncates, `POST` is a discrete
//!   device write), 200 on success.
//! - `GET dir` → its `index.html` when one exists (site behavior), otherwise
//!   a JSON listing.
//! - Errors map: `NotFound`→404, `PermissionDenied`→403, `NotSupported`→405,
//!   `InvalidArgument`→400 carrying the origin's own message (guest content
//!   validation must reach the caller), `Unreachable`→503 with a
//!   `Retry-After` hint (the provider is offline, not missing — ADR 0008),
//!   `Other`→500.

use wanix_fs::{File, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

use super::super::static_serve::content_type_for_extension;
use super::super::{HttpStatus, StaticResponse, StreamingResponse, request_header_value};
use super::GatewayResponse;

/// Bytes pulled per read while draining a sized file body.
const BODY_READ_CHUNK_BYTES: usize = 64 * 1024;

pub(super) fn get_response(
    fs: &dyn FileSystem,
    url_relative: &str,
    request: &[u8],
) -> GatewayResponse {
    let path = match origin_path(url_relative) {
        Ok(path) => path,
        Err(response) => return response.into(),
    };
    let metadata = match fs.metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => return error_response(&error).into(),
    };
    if metadata.file_type() == FileType::Directory {
        return directory_response(fs, url_relative).into();
    }
    let file = match fs.open(&path, OpenOptions::read()) {
        Ok(file) => file,
        Err(error) => return error_response(&error).into(),
    };
    if accepts_event_stream(request) {
        return GatewayResponse::Stream(StreamingResponse::sse(file));
    }
    // A sized file is a bounded read; a zero-length one is a device file whose
    // bytes only exist as they are produced (an AppFS `stream`, `#plumb/recv`)
    // — stream it chunked so a never-EOF read holds the connection live.
    match metadata.len() {
        0 => GatewayResponse::Stream(StreamingResponse::chunked(file)),
        declared => sized_body_response(file, declared, url_relative).into(),
    }
}

/// Buffers a sized file's body, bounded by its declared (stat) length.
///
/// The declared size is the trust boundary: for a mesh-mounted origin it is
/// guest-controlled (AppFS `stat` replies carry the guest's `size`), so an
/// unbounded drain-to-EOF would let a misbehaving origin declare a tiny size
/// on a never-EOF stream and grow host memory without bound while parking
/// this connection forever. Reading stops at the declared length; a file
/// that EOFs early sends the shorter honest body.
fn sized_body_response(
    mut file: Box<dyn File>,
    declared_len: u64,
    url_relative: &str,
) -> StaticResponse {
    let cap = usize::try_from(declared_len).unwrap_or(usize::MAX);
    let mut body = Vec::new();
    let mut chunk = [0u8; BODY_READ_CHUNK_BYTES];
    while body.len() < cap {
        let want = chunk.len().min(cap - body.len());
        match file.read(&mut chunk[..want]) {
            Ok(0) => break,
            Ok(read) => body.extend_from_slice(&chunk[..read]),
            Err(error) => return error_response(&error),
        }
    }
    let extension = url_relative.rsplit('/').next().and_then(|name| {
        let (_, ext) = name.rsplit_once('.')?;
        (!ext.is_empty()).then(|| ext.to_ascii_lowercase())
    });
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: content_type_for_extension(extension.as_deref()),
        headers: Vec::new(),
        body,
    }
}

/// A directory serves its `index.html` when present (the site behavior the
/// chat webapp relies on), else a JSON listing of its entries.
fn directory_response(fs: &dyn FileSystem, url_relative: &str) -> StaticResponse {
    match wanix_site_fs::read_site_file(fs, url_relative) {
        wanix_site_fs::SiteFile::Found { bytes, extension } => StaticResponse {
            status: HttpStatus::Ok,
            content_type: content_type_for_extension(extension.as_deref()),
            headers: Vec::new(),
            body: bytes,
        },
        wanix_site_fs::SiteFile::Forbidden => {
            StaticResponse::plain(HttpStatus::Forbidden, "forbidden")
        }
        wanix_site_fs::SiteFile::NotFound => directory_listing(fs, url_relative),
    }
}

fn directory_listing(fs: &dyn FileSystem, url_relative: &str) -> StaticResponse {
    let path = match origin_path(url_relative) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let entries = match fs.read_dir(&path) {
        Ok(entries) => entries,
        Err(error) => return error_response(&error),
    };
    let listing: Vec<String> = entries
        .iter()
        .map(|entry| {
            let kind = match entry.metadata().file_type() {
                FileType::Directory => "dir",
                _ => "file",
            };
            format!(
                "{{\"name\":{},\"type\":\"{kind}\",\"size\":{}}}",
                crate::json::json_string(entry.name()),
                entry.metadata().len()
            )
        })
        .collect();
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "application/json",
        headers: Vec::new(),
        body: format!("[{}]", listing.join(",")).into_bytes(),
    }
}

pub(super) fn write_response(
    fs: &dyn FileSystem,
    url_relative: &str,
    body: &[u8],
    truncate: bool,
) -> StaticResponse {
    let path = match origin_path(url_relative) {
        Ok(path) => path,
        Err(response) => return response,
    };
    // Open WITHOUT create first: in a composed origin (static dir + device
    // mount unioned at the root) a create-open would manufacture the file in
    // the first member that accepts creates, shadowing the device file behind
    // it. Only a whole-union miss falls back to creating a new file.
    let open_with = |create: bool| {
        fs.open(
            &path,
            OpenOptions {
                read: false,
                write: true,
                create,
                truncate,
            },
        )
    };
    let mut file = match open_with(false) {
        Ok(file) => file,
        Err(FsError::NotFound) => match open_with(true) {
            Ok(file) => file,
            Err(error) => return error_response(&error),
        },
        Err(error) => return error_response(&error),
    };
    let mut remaining = body;
    while !remaining.is_empty() {
        match file.write(remaining) {
            Ok(0) => {
                return StaticResponse::plain(
                    HttpStatus::InternalServerError,
                    "write made no progress",
                );
            }
            Ok(written) => remaining = &remaining[written..],
            Err(error) => return error_response(&error),
        }
    }
    StaticResponse::plain(HttpStatus::Ok, "ok\n")
}

fn origin_path(url_relative: &str) -> Result<NormalizedPath, StaticResponse> {
    let raw = match url_relative.is_empty() {
        true => ".",
        false => url_relative,
    };
    NormalizedPath::new(raw).map_err(|_| StaticResponse::plain(HttpStatus::Forbidden, "forbidden"))
}

fn accepts_event_stream(request: &[u8]) -> bool {
    request_header_value(request, "accept")
        .is_some_and(|accept| accept.to_ascii_lowercase().contains("text/event-stream"))
}

/// The gateway's `FsError`→HTTP status mapping. `Unreachable` is an outage,
/// not a miss: 503 plus a `Retry-After` hint, never 404 (ADR 0008).
fn error_response(error: &FsError) -> StaticResponse {
    match error {
        FsError::NotFound => StaticResponse::plain(HttpStatus::NotFound, "not found"),
        FsError::PermissionDenied => StaticResponse::plain(HttpStatus::Forbidden, "forbidden"),
        FsError::NotSupported => StaticResponse::plain(
            HttpStatus::MethodNotAllowed,
            "the file does not support this operation",
        ),
        FsError::Unreachable(detail) => StaticResponse::plain(
            HttpStatus::ServiceUnavailable,
            &format!("resource unreachable: {detail}"),
        )
        .with_header("Retry-After", "5"),
        FsError::InvalidPath(_) => StaticResponse::plain(HttpStatus::Forbidden, "forbidden"),
        // Content validation by the origin (e.g. an AppFS guest refusing a
        // too-long nick): a caller error with guidance, never a bare 403 —
        // the Display text carries the origin's own message.
        FsError::InvalidArgument(_) => {
            StaticResponse::plain(HttpStatus::BadRequest, &error.to_string())
        }
        FsError::IsDirectory | FsError::NotDirectory | FsError::AlreadyExists => {
            StaticResponse::plain(HttpStatus::Conflict, "conflict")
        }
        _ => StaticResponse::plain(HttpStatus::InternalServerError, "internal server error"),
    }
}
