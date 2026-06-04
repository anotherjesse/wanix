use std::path::Path;

use crate::json::json_string;
use crate::qemu::DEFAULT_P9_MSIZE;

use super::ServeRoots;
use super::boot::{first_executable_init_route, first_existing_static_route};
use super::http::{HttpStatus, StaticResponse};

pub(super) const DIRECT_V86_BUNDLE: &str = "direct-v86";
pub(super) const DIRECT_V86_DEFAULT_CMDLINE: &str = "console=hvc0 init=/bin/init rw root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L,aname=,cache=none,msize=131072 loglevel=3";
pub(super) const DIRECT_V86_DEFAULT_P9_MSIZE: u32 = DEFAULT_P9_MSIZE;
pub(super) const DIRECT_V86_DEFAULT_KERNEL_PATH: &str = "/boot/bzImage";
const DIRECT_V86_INIT_PATH: &str = "/bin/init";
pub(super) const DIRECT_V86_MEMORY_SIZE: u32 = 1024 * 1024 * 1024;
pub(super) const DIRECT_V86_VGA_MEMORY_SIZE: u32 = 8 * 1024 * 1024;
pub(super) const DIRECT_V86_MODULE_PATH: &str = "/v86/lib/libv86.mjs";
pub(super) const DIRECT_V86_MOD_REEXPORT_PATH: &str = "/v86/lib/mod.js";
pub(super) const DIRECT_V86_OFFSCREEN_PATH: &str = "/v86/lib/offscreen.js";
pub(super) const DIRECT_V86_WASM_PATH: &str = "/v86/bundle/v86.wasm";
pub(super) const DIRECT_V86_BIOS_PATH: &str = "/v86/bundle/seabios.bin";
pub(super) const DIRECT_V86_VGA_BIOS_PATH: &str = "/v86/bundle/vgabios.bin";
const DIRECT_V86_KERNEL_CANDIDATES: &[&str] = &[DIRECT_V86_DEFAULT_KERNEL_PATH, "/bzImage"];
const DIRECT_V86_INITRD_CANDIDATES: &[&str] =
    &["/boot/initrd", "/boot/initrd.img", "/initrd", "/initrd.img"];

struct BuiltinAsset {
    route: &'static str,
    content_type: &'static str,
    bytes: &'static [u8],
}

const DIRECT_V86_ASSETS: &[BuiltinAsset] = &[
    BuiltinAsset {
        route: DIRECT_V86_MOD_REEXPORT_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../../v86/lib/mod.js"),
    },
    BuiltinAsset {
        route: DIRECT_V86_MODULE_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../../v86/lib/libv86.mjs"),
    },
    BuiltinAsset {
        route: DIRECT_V86_OFFSCREEN_PATH,
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../../../v86/lib/offscreen.js"),
    },
    BuiltinAsset {
        route: DIRECT_V86_WASM_PATH,
        content_type: "application/wasm",
        bytes: include_bytes!("../../../../v86/bundle/v86.wasm"),
    },
    BuiltinAsset {
        route: DIRECT_V86_BIOS_PATH,
        content_type: "application/octet-stream",
        bytes: include_bytes!("../../../../v86/bundle/seabios.bin"),
    },
    BuiltinAsset {
        route: DIRECT_V86_VGA_BIOS_PATH,
        content_type: "application/octet-stream",
        bytes: include_bytes!("../../../../v86/bundle/vgabios.bin"),
    },
];

pub(super) fn direct_v86_asset_response(
    roots: &ServeRoots,
    relative_path: &Path,
) -> Option<StaticResponse> {
    if roots.bundle.as_deref() != Some(DIRECT_V86_BUNDLE) {
        return None;
    }
    let route = format!("/{}", relative_path.to_str()?);
    let asset = DIRECT_V86_ASSETS
        .iter()
        .find(|asset| asset.route == route)?;
    Some(StaticResponse {
        status: HttpStatus::Ok,
        content_type: asset.content_type,
        body: asset.bytes.to_vec(),
    })
}

pub(super) struct RootfsHandoffReadiness {
    pub(super) missing: Vec<&'static str>,
}

pub(super) fn rootfs_handoff_readiness(static_root: &Path) -> RootfsHandoffReadiness {
    let kernel = first_existing_static_route(static_root, DIRECT_V86_KERNEL_CANDIDATES);
    let init = first_executable_init_route(static_root, DIRECT_V86_INIT_PATH);
    let mut missing = Vec::new();
    if kernel.is_none() {
        missing.push(DIRECT_V86_DEFAULT_KERNEL_PATH);
    }
    if init.is_none() {
        missing.push(DIRECT_V86_INIT_PATH);
    }
    RootfsHandoffReadiness { missing }
}

pub(super) fn direct_v86_boot_json(static_root: &Path) -> String {
    let readiness = rootfs_handoff_readiness(static_root);
    let mut fields = Vec::new();
    let kernel = first_existing_static_route(static_root, DIRECT_V86_KERNEL_CANDIDATES);
    let initrd = first_existing_static_route(static_root, DIRECT_V86_INITRD_CANDIDATES);
    let init = first_executable_init_route(static_root, DIRECT_V86_INIT_PATH);
    if let Some(kernel) = kernel {
        fields.push(format!("\"kernel\":{}", json_string(kernel)));
    }
    if let Some(initrd) = initrd {
        fields.push(format!("\"initrd\":{}", json_string(initrd)));
    }
    if let Some(init) = init {
        fields.push(format!("\"init\":{}", json_string(init)));
    }
    fields.push(format!("\"ready\":{}", readiness.missing.is_empty()));
    if !readiness.missing.is_empty() {
        let missing = readiness
            .missing
            .into_iter()
            .map(json_string)
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"missing\":[{missing}]"));
    }
    format!("{{{}}}", fields.join(","))
}
