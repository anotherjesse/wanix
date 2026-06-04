use std::net::SocketAddr;
use std::path::{Component, Path};

use super::super::direct_v86::{DIRECT_V86_BUNDLE, direct_v86_asset_response};
use super::super::discovery::{rootfs_handoff_response, serve_discovery_response};
use super::super::html::{direct_v86_bundle_html, fs9p_bundle_html, workbench_fs9p_bundle_html};
use super::super::{FS9P_BUNDLE, ServeRoots, WORKBENCH_FS9P_BUNDLE};
use super::{HttpStatus, StaticResponse, request_target};

pub(super) fn http_route_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
) -> Option<StaticResponse> {
    well_known_response(roots, relative_path, request, peer_addr)
        .or_else(|| bundle_response(roots, relative_path, request))
        .or_else(|| direct_v86_asset_response(roots, relative_path))
}

fn well_known_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
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
                Some(serve_discovery_response(roots, request, peer_addr))
            }
        }
        Some(Component::Normal(component)) if component == "rootfs.json" => {
            if has_extra_components {
                Some(StaticResponse::plain(HttpStatus::NotFound, "not found"))
            } else {
                Some(rootfs_handoff_response(roots, peer_addr))
            }
        }
        Some(Component::Normal(component)) if component == "export9p" && !has_extra_components => {
            Some(StaticResponse::plain(
                HttpStatus::BadRequest,
                "websocket upgrade required",
            ))
        }
        Some(Component::Normal(component))
            if component == "qjs-shell" && !has_extra_components && roots.wanix_services =>
        {
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
    if relative_path != Path::new("index.html") {
        return None;
    }
    let target = request_target(request)?;
    let (_, query) = target.split_once('?')?;
    match (roots.bundle.as_deref(), query_param(query, "bundle")) {
        (Some(DIRECT_V86_BUNDLE), Some(DIRECT_V86_BUNDLE)) => Some(direct_v86_bundle_response()),
        (Some(FS9P_BUNDLE), Some(FS9P_BUNDLE)) => Some(fs9p_bundle_response()),
        (Some(WORKBENCH_FS9P_BUNDLE), Some(WORKBENCH_FS9P_BUNDLE)) => {
            Some(workbench_fs9p_bundle_response())
        }
        _ => None,
    }
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

fn fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: fs9p_bundle_html().into_bytes(),
    }
}

fn workbench_fs9p_bundle_response() -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        body: workbench_fs9p_bundle_html().into_bytes(),
    }
}
