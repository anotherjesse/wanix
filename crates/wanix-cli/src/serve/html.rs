use crate::json::json_string;

use super::direct_v86::{
    DIRECT_V86_BUNDLE, DIRECT_V86_DEFAULT_CMDLINE, DIRECT_V86_DEFAULT_KERNEL_PATH,
    DIRECT_V86_MEMORY_SIZE, DIRECT_V86_VGA_MEMORY_SIZE,
};
use super::{FS9P_BUNDLE, WORKBENCH_FS9P_BUNDLE};

const DIRECT_V86_HTML: &str = include_str!("direct_v86.html");
const FS9P_HTML: &str = include_str!("fs9p.html");
const WORKBENCH_FS9P_HTML: &str = include_str!("workbench_fs9p.html");

pub(super) fn bundle_html(bundle: &str) -> Option<String> {
    match bundle {
        DIRECT_V86_BUNDLE => Some(direct_v86_bundle_html()),
        FS9P_BUNDLE => Some(fs9p_bundle_html()),
        WORKBENCH_FS9P_BUNDLE => Some(workbench_fs9p_bundle_html()),
        _ => None,
    }
}

fn direct_v86_bundle_html() -> String {
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

fn fs9p_bundle_html() -> String {
    FS9P_HTML.to_owned()
}

fn workbench_fs9p_bundle_html() -> String {
    WORKBENCH_FS9P_HTML.to_owned()
}
