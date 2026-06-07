use std::net::SocketAddr;
use std::path::Path;

use crate::json::json_string;
use crate::rootfs::rootfs_json_handoff_for_prepared_root;

use super::ServeRoots;
use super::direct_v86::{
    DIRECT_V86_BIOS_PATH, DIRECT_V86_DEFAULT_CMDLINE, DIRECT_V86_DEFAULT_P9_MSIZE,
    DIRECT_V86_MEMORY_SIZE, DIRECT_V86_MOD_REEXPORT_PATH, DIRECT_V86_MODULE_PATH,
    DIRECT_V86_OFFSCREEN_PATH, DIRECT_V86_VGA_BIOS_PATH, DIRECT_V86_VGA_MEMORY_SIZE,
    DIRECT_V86_WASM_PATH, direct_v86_boot_json, rootfs_handoff_readiness,
};
use super::http::app::app_route_json;
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
    let app_url = format!("http://{host}/.wanix/app/{{name}}");
    let ethernet_url = format!("ws://{host}/.well-known/ethernet");
    let bundle = roots
        .bundle
        .as_deref()
        .map(json_string)
        .unwrap_or_else(|| "null".to_owned());
    let services = serve_services_json(roots);
    let qjs_shell_route = serve_qjs_shell_route_json(roots, &qjs_shell_url);
    let app_route = app_route_json(roots, &app_url);
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
         \"httpApp\":{},\
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
        app_route,
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
    if roots.wanix_services {
        "{\"task\":\"#task\",\"term\":\"#term\",\"drivers\":[\"noop\",\"qjs\",\"wasm\"]}".to_owned()
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
             \"sessionMessage\":\"{{\\\"type\\\":\\\"session\\\",\\\"protocol\\\":\\\"wanix-qjs-shell.v1\\\",\\\"taskId\\\":\\\"ID\\\",\\\"terminalId\\\":\\\"ID\\\",\\\"cwd\\\":\\\"PATH\\\"}}\",\
             \"mutationMessage\":\"{{\\\"type\\\":\\\"mutation\\\",\\\"protocol\\\":\\\"wanix-qjs-shell.v1\\\",\\\"taskId\\\":\\\"ID\\\",\\\"terminalId\\\":\\\"ID\\\",\\\"cwd\\\":\\\"PATH\\\",\\\"paths\\\":[\\\"/path\\\"],\\\"operations\\\":[{{\\\"kind\\\":\\\"write\\\",\\\"command\\\":\\\"write /path data\\\",\\\"status\\\":\\\"changed\\\",\\\"target\\\":\\\"/path\\\",\\\"evidence\\\":\\\"qjs-shell-command-record\\\",\\\"terminalOutput\\\":\\\"wrote /path\\\",\\\"outcome\\\":{{\\\"status\\\":\\\"ok\\\",\\\"changed\\\":true,\\\"evidence\\\":\\\"qjs-shell-command-record\\\",\\\"terminalOutput\\\":\\\"wrote /path\\\"}},\\\"paths\\\":[\\\"/path\\\"]}}]}}\",\
             \"unchangedOperationMessage\":\"{{\\\"type\\\":\\\"mutation\\\",\\\"protocol\\\":\\\"wanix-qjs-shell.v1\\\",\\\"taskId\\\":\\\"ID\\\",\\\"terminalId\\\":\\\"ID\\\",\\\"cwd\\\":\\\"PATH\\\",\\\"paths\\\":[],\\\"operations\\\":[{{\\\"kind\\\":\\\"rm\\\",\\\"command\\\":\\\"rm missing\\\",\\\"status\\\":\\\"unchanged\\\",\\\"target\\\":\\\"/missing\\\",\\\"evidence\\\":\\\"qjs-shell-command-record\\\",\\\"diagnostic\\\":\\\"rm: missing: errno -44\\\",\\\"terminalOutput\\\":\\\"rm: missing: errno -44\\\",\\\"outcome\\\":{{\\\"status\\\":\\\"error\\\",\\\"changed\\\":false,\\\"diagnostic\\\":\\\"rm: missing: errno -44\\\",\\\"evidence\\\":\\\"qjs-shell-command-record\\\",\\\"terminalOutput\\\":\\\"rm: missing: errno -44\\\"}},\\\"paths\\\":[]}}]}}\",\
             \"exitMessage\":\"{{\\\"type\\\":\\\"exit\\\",\\\"code\\\":N}}\",\
             \"terminalLifecycle\":\"owned-resource-closed-on-session-close\"}}",
            json_string(websocket_url)
        )
    } else {
        "{\"status\":\"disabled\"}".to_owned()
    }
}
