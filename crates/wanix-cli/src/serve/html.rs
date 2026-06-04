use crate::json::json_string;

use super::direct_v86::{
    DIRECT_V86_DEFAULT_CMDLINE, DIRECT_V86_DEFAULT_KERNEL_PATH, DIRECT_V86_MEMORY_SIZE,
    DIRECT_V86_VGA_MEMORY_SIZE,
};

const DIRECT_V86_HTML: &str = include_str!("direct_v86.html");
const FS9P_HTML: &str = include_str!("fs9p.html");
const WORKBENCH_FS9P_HTML: &str = include_str!("workbench_fs9p.html");

pub(super) fn direct_v86_bundle_html() -> String {
    DIRECT_V86_HTML
        .replace(
            "__WANIX_DEFAULT_CMDLINE_JSON__",
            &json_string(DIRECT_V86_DEFAULT_CMDLINE),
        )
        .replace(
            "__WANIX_DEFAULT_KERNEL_URL_JSON__",
            &json_string(DIRECT_V86_DEFAULT_KERNEL_PATH),
        )
        .replace(
            "__WANIX_DEFAULT_MEMORY_SIZE__",
            &DIRECT_V86_MEMORY_SIZE.to_string(),
        )
        .replace(
            "__WANIX_DEFAULT_VGA_MEMORY_SIZE__",
            &DIRECT_V86_VGA_MEMORY_SIZE.to_string(),
        )
}

pub(super) fn fs9p_bundle_html() -> String {
    FS9P_HTML.to_owned()
}

pub(super) fn workbench_fs9p_bundle_html() -> String {
    WORKBENCH_FS9P_HTML.to_owned()
}
