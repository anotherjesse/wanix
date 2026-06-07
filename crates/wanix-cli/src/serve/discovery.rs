use std::net::SocketAddr;
use std::path::Path;

use crate::json::{json_string, json_string_array};
use crate::rootfs::rootfs_json_handoff_for_prepared_root;

use super::ServeRoots;
use super::direct_v86::{
    DIRECT_V86_BIOS_PATH, DIRECT_V86_DEFAULT_CMDLINE, DIRECT_V86_DEFAULT_P9_MSIZE,
    DIRECT_V86_MEMORY_SIZE, DIRECT_V86_MOD_REEXPORT_PATH, DIRECT_V86_MODULE_PATH,
    DIRECT_V86_OFFSCREEN_PATH, DIRECT_V86_VGA_BIOS_PATH, DIRECT_V86_VGA_MEMORY_SIZE,
    DIRECT_V86_WASM_PATH, direct_v86_boot_json, rootfs_handoff_readiness,
};
use super::http::{HttpStatus, StaticResponse};
use super::terminal_ws::QJS_SHELL_WEBSOCKET_PATH;

mod host;
pub(super) use host::display_host;
use host::{is_loopback_peer, request_host};

pub(super) fn serve_discovery_response(
    roots: &ServeRoots,
    request: &[u8],
    peer_addr: SocketAddr,
) -> StaticResponse {
    StaticResponse {
        status: HttpStatus::Ok,
        content_type: "application/json",
        headers: Vec::new(),
        body: serve_discovery_json(roots, request, peer_addr).into_bytes(),
    }
}

pub(super) fn serve_discovery_json(
    roots: &ServeRoots,
    request: &[u8],
    peer_addr: SocketAddr,
) -> String {
    let host = request_host(request).unwrap_or_else(|| display_host(roots.local_addr));
    let p9_url = format!("ws://{host}/.well-known/export9p");
    let rootfs_url = format!("http://{host}/.well-known/rootfs.json");
    let qjs_shell_url = format!("ws://{host}{QJS_SHELL_WEBSOCKET_PATH}");
    let ethernet_url = format!("ws://{host}/.well-known/ethernet");
    let bundle = roots
        .bundle
        .as_deref()
        .map(json_string)
        .unwrap_or_else(|| "null".to_owned());
    let services = serve_services_json(roots);
    let qjs_shell_route = serve_qjs_shell_route_json(roots, &qjs_shell_url);
    let rootfs_route = serve_rootfs_route_json(&roots.static_root, &rootfs_url, peer_addr);
    let direct_v86_boot = direct_v86_boot_json(&roots.static_root);
    format!(
        "{{\"version\":1,\
         \"runtime\":\"wanix-rust\",\
         \"routes\":{{\
         \"p9\":{{\"websocket\":{},\"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\",\
         \"supportedProtocols\":[\"9P2000.L\",\"9P2000.L.Google.2\"]}},\
         \"rootfs\":{},\
         \"qjsShell\":{},\
         \"ethernet\":{{\"websocket\":{},\"status\":\"not-implemented\"}}\
         }},\
         \"v86\":{{\"assets\":{{\"module\":{},\"mod\":{},\"offscreen\":{},\"wasm\":{},\"bios\":{},\"vgaBios\":{}}},\
         \"boot\":{},\
         \"defaultCmdline\":{},\"p9Msize\":{},\"memorySize\":{},\"vgaMemorySize\":{},\"virtioConsole\":true}},\
         \"services\":{},\
         \"bundle\":{}}}",
        json_string(&p9_url),
        rootfs_route,
        qjs_shell_route,
        json_string(&ethernet_url),
        json_string(DIRECT_V86_MODULE_PATH),
        json_string(DIRECT_V86_MOD_REEXPORT_PATH),
        json_string(DIRECT_V86_OFFSCREEN_PATH),
        json_string(DIRECT_V86_WASM_PATH),
        json_string(DIRECT_V86_BIOS_PATH),
        json_string(DIRECT_V86_VGA_BIOS_PATH),
        direct_v86_boot,
        json_string(DIRECT_V86_DEFAULT_CMDLINE),
        DIRECT_V86_DEFAULT_P9_MSIZE,
        DIRECT_V86_MEMORY_SIZE,
        DIRECT_V86_VGA_MEMORY_SIZE,
        services,
        bundle
    )
}

pub(super) fn rootfs_handoff_response(roots: &ServeRoots, peer_addr: SocketAddr) -> StaticResponse {
    if !is_loopback_peer(peer_addr) {
        return StaticResponse::plain(
            HttpStatus::Forbidden,
            "rootfs handoff is available only to loopback clients",
        );
    }
    match rootfs_json_handoff_for_prepared_root(&roots.static_root) {
        Ok(body) => StaticResponse {
            status: HttpStatus::Ok,
            content_type: "application/json",
            headers: Vec::new(),
            body: body.into_bytes(),
        },
        Err(error) if error.exit_code() == 2 => {
            StaticResponse::plain(HttpStatus::BadRequest, &error.to_string())
        }
        Err(error) => StaticResponse::plain(HttpStatus::Conflict, &error.to_string()),
    }
}

fn serve_rootfs_route_json(static_root: &Path, url: &str, peer_addr: SocketAddr) -> String {
    let readiness = rootfs_handoff_readiness(static_root);
    let loopback = is_loopback_peer(peer_addr);
    let mut ready = readiness.missing.is_empty();
    let mut error = None;
    let status = if !loopback {
        "local-only"
    } else if readiness.missing.is_empty() {
        match rootfs_json_handoff_for_prepared_root(static_root) {
            Ok(_) => "available",
            Err(manifest_error) => {
                ready = false;
                error = Some(manifest_error.to_string());
                "invalid"
            }
        }
    } else {
        "unprepared"
    };
    let mut fields = vec![
        format!("\"url\":{}", json_string(url)),
        "\"kind\":\"wanix-rootfs.v1\"".to_owned(),
        format!("\"status\":{}", json_string(status)),
        format!("\"ready\":{}", ready),
    ];
    if !readiness.missing.is_empty() {
        let missing = readiness
            .missing
            .iter()
            .map(|route| json_string(route))
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"missing\":[{missing}]"));
    }
    if let Some(error) = error {
        fields.push(format!("\"error\":{}", json_string(&error)));
    }
    format!("{{{}}}", fields.join(","))
}

fn serve_services_json(roots: &ServeRoots) -> String {
    // Advertise the drivers derived from the registry (`serve_task_table`) so the
    // discovery JSON cannot silently drift from what clients can actually launch
    // via `#task/new/<kind>`. Today this is `["auto","noop","qjs","wasm"]`.
    if roots.wanix_services {
        let devices: Vec<String> = super::roots::INSPECTABLE_SERVICE_DEVICES
            .iter()
            .map(|device| (*device).to_owned())
            .collect();
        format!(
            "{{\"task\":\"#task\",\"term\":\"#term\",\"drivers\":{},\"devices\":{}}}",
            json_string_array(&roots.driver_kinds),
            json_string_array(&devices)
        )
    } else {
        "null".to_owned()
    }
}

fn serve_qjs_shell_route_json(roots: &ServeRoots, websocket_url: &str) -> String {
    if roots.wanix_services {
        format!(
            "{{\"websocket\":{},\"protocol\":\"wanix-qjs-shell.v1\",\
             \"mode\":\"raw-bytes\",\"status\":\"available\",\
             \"cwdQuery\":\"cwd\",\"defaultCwd\":\".\",\
             \"resize\":[\"{{\\\"type\\\":\\\"resize\\\",\\\"columns\\\":COLS,\\\"rows\\\":ROWS}}\",\
             \"resize COLS ROWS\"],\
             \"exitMessage\":\"{{\\\"type\\\":\\\"exit\\\",\\\"code\\\":N}}\",\
             \"terminalLifecycle\":\"owned-resource-closed-on-session-close\"}}",
            json_string(websocket_url)
        )
    } else {
        "{\"status\":\"disabled\"}".to_owned()
    }
}
