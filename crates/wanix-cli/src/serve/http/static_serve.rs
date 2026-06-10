//! Static-file serving helpers shared by the FS-backed site path and the
//! disk-backed workbench asset path.

use std::fs;
use std::path::Path;

use super::{HttpStatus, StaticResponse};

/// Serves the static path through a [`wanix_fs::FileSystem`] (Phase 0).
///
/// All static-file I/O goes through the filesystem trait, not `std::fs`:
/// directory-index resolution, path confinement, and the byte read are owned by
/// [`wanix_site_fs::read_site_file`]. The `--root DIR` path passes a `LocalFs`
/// here, so its behavior is unchanged; other phases serve other filesystems
/// (an in-memory generator output via `#sites`, a CAS snapshot).
pub(in crate::serve) fn read_static_response(
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
pub(in crate::serve) fn read_disk_static_response(
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
pub(in crate::serve) fn path_to_url_relative(relative_path: &Path) -> String {
    relative_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn content_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|extension| extension.to_str());
    content_type_for_extension(extension)
}

/// Maps a (lowercased) file extension to a static content type. Keys on the
/// extension only, so it works identically for a host `Path` and a virtual
/// filesystem path. Shared with the WebDoor gateway's file mapping.
pub(in crate::serve) fn content_type_for_extension(extension: Option<&str>) -> &'static str {
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
