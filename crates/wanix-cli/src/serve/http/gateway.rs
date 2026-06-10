//! The WebDoor request router: HTTP requests against bound gateway origins.
//!
//! Every `--bind` origin is one composed namespace ([`super::super::webdoor`])
//! addressed by `Host`-header routing. This module decides *which* origin (or
//! the bare-host index / unknown-name 404) answers; the verb→filesystem
//! mapping itself lives in [`files`].
//!
//! Identity honesty: requests here carry **no principal**. Whatever the origin
//! namespace reaches over the mesh sees the gateway's own dialer key as the
//! one principal for every browser user (see the webdoor module docs).

mod files;

use wanix_fs::FileSystem;
use wanix_sites::Host;

use super::super::ServeRoots;
use super::{
    HttpStatus, StaticResponse, StreamingResponse, gateway_request_path, path_to_url_relative,
    request_body, request_header_value, request_method,
};

/// A gateway answer: either a buffered response or a live-pumped body.
pub(in crate::serve) enum GatewayResponse {
    Static(StaticResponse),
    Stream(StreamingResponse),
}

impl From<StaticResponse> for GatewayResponse {
    fn from(response: StaticResponse) -> Self {
        Self::Static(response)
    }
}

/// Routes a request through the WebDoor. `None` falls through to the classic
/// serve behavior (routes, `#sites`, `--root`); the door never engages when no
/// `--bind` was given.
pub(in crate::serve) fn gateway_response(
    roots: &ServeRoots,
    request: &[u8],
) -> Option<GatewayResponse> {
    if roots.webdoor.is_empty() {
        return None;
    }
    let raw_host = request_header_value(request, "host");
    let Some(host) = raw_host.and_then(Host::parse) else {
        return bare_host_index(roots, request).map(GatewayResponse::Static);
    };
    match roots.webdoor.resolve(&host) {
        Some(fs) => Some(origin_response(fs.as_ref(), &host, request)),
        None => unknown_host_response(roots, &host).map(GatewayResponse::Static),
    }
}

/// `GET /` on the bare host (`localhost`, an IP, no `Host` header) serves an
/// index of the bound names; every other bare-host request keeps the classic
/// behavior (discovery, bundles, `--root` statics).
fn bare_host_index(roots: &ServeRoots, request: &[u8]) -> Option<StaticResponse> {
    if request_method(request).as_deref() != Some("GET") {
        return None;
    }
    if !gateway_request_path(request).is_ok_and(|path| path.as_os_str().is_empty()) {
        return None;
    }
    let port = roots.local_addr.port();
    let mut html =
        String::from("<!doctype html><title>wanix gateway</title><h1>bound names</h1><ul>");
    for name in roots.webdoor.names() {
        html.push_str(&format!(
            "<li><a href=\"http://{name}:{port}/\">{name}</a></li>"
        ));
    }
    html.push_str("</ul>");
    Some(StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        headers: Vec::new(),
        body: html.into_bytes(),
    })
}

/// A named host the door does not know: 404 pointing at the bare-host index —
/// unless `#sites` binds it (services mode), which falls through to the static
/// site gateway.
fn unknown_host_response(roots: &ServeRoots, host: &Host) -> Option<StaticResponse> {
    if roots.wanix_services && roots.sites.resolve(host).is_some() {
        return None;
    }
    Some(StaticResponse::plain(
        HttpStatus::NotFound,
        &format!(
            "unknown gateway name; the bound names are listed at http://localhost:{}/",
            roots.local_addr.port()
        ),
    ))
}

fn origin_response(fs: &dyn FileSystem, host: &Host, request: &[u8]) -> GatewayResponse {
    let path = match gateway_request_path(request) {
        Ok(path) => path,
        Err(status) => return StaticResponse::plain(status, status.reason()).into(),
    };
    let url_relative = path_to_url_relative(&path);
    match request_method(request).as_deref() {
        Some("GET") => files::get_response(fs, &url_relative, request),
        Some(method @ ("POST" | "PUT")) => {
            if is_cross_site_write(request, host) {
                return StaticResponse::plain(
                    HttpStatus::Forbidden,
                    "cross-site write refused: gateway origins accept writes only from \
                     their own pages (or Origin-less native tools)",
                )
                .into();
            }
            files::write_response(fs, &url_relative, request_body(request), method == "PUT").into()
        }
        _ => StaticResponse::plain(
            HttpStatus::MethodNotAllowed,
            "gateway origins accept GET, POST, and PUT",
        )
        .into(),
    }
}

/// Whether a write request was sent by a browser page from *another* origin.
///
/// CORS never blocks a "simple" cross-site POST from being sent, so the
/// loopback-only WebDoor would still take drive-by writes from any web page
/// the operator's browser visits. Browsers stamp every POST/PUT with the
/// page's `Origin`; one naming anything but this bound gateway name is
/// refused. Requests without an `Origin` (curl, native tools, same-origin
/// GET-style navigations) pass — they are not browser-mediated cross-site
/// requests.
fn is_cross_site_write(request: &[u8], host: &Host) -> bool {
    let Some(origin) = request_header_value(request, "origin") else {
        return false;
    };
    let origin = origin.trim();
    let Some(origin_host) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        // An opaque ("null") or non-HTTP origin is never this gateway page.
        return true;
    };
    let origin_host = origin_host.split('/').next().unwrap_or(origin_host);
    Host::parse(origin_host).is_none_or(|origin_host| origin_host != *host)
}
