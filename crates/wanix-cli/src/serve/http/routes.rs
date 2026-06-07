use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use super::super::ServeRoots;
use super::super::WORKBENCH_FS9P_BUNDLE;
use super::super::direct_v86::direct_v86_asset_response;
use super::super::discovery::{rootfs_handoff_response, serve_discovery_response};
use super::super::html::bundle_html;
use super::app::app_route_response;
use super::{HttpStatus, StaticResponse, read_static_response, request_target};

const LOCAL_BUNDLE_CACHE_CONTROL: &str = "no-store";

pub(super) fn http_route_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
) -> Option<StaticResponse> {
    well_known_response(roots, relative_path, request, peer_addr)
        .or_else(|| bundle_response(roots, relative_path, request))
        .or_else(|| app_route_response(roots, relative_path, request, peer_addr))
        .or_else(|| workbench_asset_response(roots, relative_path))
        .or_else(|| direct_v86_asset_response(roots, relative_path))
}

// Serve the workbench bundle's static assets (vscode-web under workbench/code,
// the compiled extension under workbench/dist, media, etc.) from the repo's
// `workbench/` tree rather than the served user root, so a disposable root can
// still boot the cockpit. Restored from the origin/rust cockpit work; no-store
// keeps the browser from caching a stale extension.js across rebuilds.
fn workbench_asset_response(roots: &ServeRoots, relative_path: &Path) -> Option<StaticResponse> {
    if roots.bundle.as_deref() != Some(WORKBENCH_FS9P_BUNDLE) {
        return None;
    }
    let asset_path = workbench_asset_path(relative_path)?;
    let asset_root = workbench_asset_root()?;
    Some(
        read_static_response(&asset_root, asset_path)
            .with_header("Cache-Control", LOCAL_BUNDLE_CACHE_CONTROL),
    )
}

fn workbench_asset_path(relative_path: &Path) -> Option<&Path> {
    relative_path.strip_prefix("workbench").ok()
}

fn workbench_asset_root() -> Option<PathBuf> {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workbench")).ok()
}

enum WellKnownEndpoint {
    WanixDiscovery,
    RootfsHandoff,
    Export9pWebSocket,
    QjsShellWebSocket,
    EthernetWebSocket,
    NotFound,
}

fn well_known_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
) -> Option<StaticResponse> {
    let endpoint = well_known_endpoint(roots, relative_path)?;
    Some(well_known_endpoint_response(
        roots, request, peer_addr, endpoint,
    ))
}

fn well_known_endpoint_response(
    roots: &ServeRoots,
    request: &[u8],
    peer_addr: SocketAddr,
    endpoint: WellKnownEndpoint,
) -> StaticResponse {
    match endpoint {
        WellKnownEndpoint::WanixDiscovery => serve_discovery_response(roots, request, peer_addr),
        WellKnownEndpoint::RootfsHandoff => rootfs_handoff_response(roots, peer_addr),
        endpoint => static_well_known_response(endpoint),
    }
}

fn static_well_known_response(endpoint: WellKnownEndpoint) -> StaticResponse {
    match endpoint {
        WellKnownEndpoint::Export9pWebSocket | WellKnownEndpoint::QjsShellWebSocket => {
            websocket_upgrade_required_response()
        }
        WellKnownEndpoint::EthernetWebSocket => ethernet_not_implemented_response(),
        _ => not_found_response(),
    }
}

fn well_known_endpoint(roots: &ServeRoots, relative_path: &Path) -> Option<WellKnownEndpoint> {
    let mut components = relative_path.components();
    match components.next() {
        Some(Component::Normal(component)) if component == ".well-known" => {}
        _ => return None,
    }
    if components.clone().nth(1).is_some() {
        return Some(WellKnownEndpoint::NotFound);
    }
    let Some(Component::Normal(endpoint)) = components.next() else {
        return Some(WellKnownEndpoint::NotFound);
    };
    Some(named_well_known_endpoint(endpoint, roots.wanix_services))
}

fn named_well_known_endpoint(
    endpoint: &std::ffi::OsStr,
    wanix_services: bool,
) -> WellKnownEndpoint {
    manifest_endpoint(endpoint)
        .or_else(|| websocket_endpoint(endpoint, wanix_services))
        .unwrap_or(WellKnownEndpoint::NotFound)
}

fn manifest_endpoint(endpoint: &std::ffi::OsStr) -> Option<WellKnownEndpoint> {
    if endpoint == "wanix.json" {
        return Some(WellKnownEndpoint::WanixDiscovery);
    }
    if endpoint == "rootfs.json" {
        return Some(WellKnownEndpoint::RootfsHandoff);
    }
    None
}

fn websocket_endpoint(
    endpoint: &std::ffi::OsStr,
    wanix_services: bool,
) -> Option<WellKnownEndpoint> {
    if endpoint == "export9p" {
        return Some(WellKnownEndpoint::Export9pWebSocket);
    }
    if endpoint == "qjs-shell" && wanix_services {
        return Some(WellKnownEndpoint::QjsShellWebSocket);
    }
    if endpoint == "ethernet" {
        return Some(WellKnownEndpoint::EthernetWebSocket);
    }
    None
}

fn websocket_upgrade_required_response() -> StaticResponse {
    StaticResponse::plain(HttpStatus::BadRequest, "websocket upgrade required")
}

fn ethernet_not_implemented_response() -> StaticResponse {
    StaticResponse::plain(
        HttpStatus::NotImplemented,
        "ethernet websocket bridge is not implemented in rust serve",
    )
}

fn not_found_response() -> StaticResponse {
    StaticResponse::plain(HttpStatus::NotFound, "not found")
}

fn bundle_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
) -> Option<StaticResponse> {
    let (configured_bundle, requested_bundle) = bundle_request(roots, relative_path, request)?;
    matching_bundle_response(configured_bundle, requested_bundle)
}

fn bundle_request<'a>(
    roots: &'a ServeRoots,
    relative_path: &Path,
    request: &'a [u8],
) -> Option<(&'a str, &'a str)> {
    require_bundle_index_path(relative_path)?;
    Some((configured_bundle(roots)?, requested_bundle(request)?))
}

fn require_bundle_index_path(relative_path: &Path) -> Option<()> {
    if relative_path != Path::new("index.html") {
        return None;
    }
    Some(())
}

fn configured_bundle(roots: &ServeRoots) -> Option<&str> {
    roots.bundle.as_deref()
}

fn requested_bundle(request: &[u8]) -> Option<&str> {
    let target = request_target(request)?;
    let (_, query) = target.split_once('?')?;
    query_param(query, "bundle")
}

fn matching_bundle_response(
    configured_bundle: &str,
    requested_bundle: &str,
) -> Option<StaticResponse> {
    if configured_bundle != requested_bundle {
        return None;
    }
    bundle_html_response(configured_bundle)
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

fn bundle_html_response(bundle: &str) -> Option<StaticResponse> {
    let html = bundle_html(bundle)?;
    Some(StaticResponse {
        status: HttpStatus::Ok,
        content_type: "text/html; charset=utf-8",
        headers: Vec::new(),
        body: html.into_bytes(),
    })
}
