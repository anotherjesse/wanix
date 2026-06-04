use wanix_task::quote_cmd_argv;

use crate::CliError;
use crate::json::{json_string, json_string_array};
use crate::qemu::{DEFAULT_P9_MSIZE, qemu_default_json_handoff_for_root};

use super::{INIT_PATH, RootfsReport};

pub(super) fn rootfs_text_handoff(report: &RootfsReport) -> String {
    let out = report.out_path.to_string_lossy().into_owned();
    let qemu = quote_cmd_argv(["wanix-rust", "qemu", "--root", &out, "--exec"]);
    let serve = quote_cmd_argv([
        "wanix-rust",
        "serve",
        &out,
        "--bundle",
        "direct-v86",
        "--wanix-services",
    ]);
    format!(
        "rootfs extracted to {}\n\
         kernel /{}\n\
         init /{}\n\
         qemu {qemu}\n\
         serve {serve}\n",
        report.out_path.display(),
        report.kernel_route,
        INIT_PATH,
    )
}

pub(super) fn rootfs_json_handoff(report: &RootfsReport) -> Result<String, CliError> {
    let out = report.out_path.to_string_lossy().into_owned();
    let kernel_route = format!("/{}", report.kernel_route);
    let init_route = format!("/{INIT_PATH}");
    let kernel_path = report.out_path.join(report.kernel_route);
    let init_path = report.out_path.join(INIT_PATH);
    let qemu = qemu_default_json_handoff_for_root(&report.out_path)?;
    let serve_argv = [
        "wanix-rust",
        "serve",
        out.as_str(),
        "--bundle",
        "direct-v86",
        "--wanix-services",
    ];
    Ok(format!(
        "{{\n  \"kind\":\"wanix-rootfs.v1\",\n  \"rootPath\":{},\n  \"kernelRoute\":{},\n  \"kernelPath\":{},\n  \"initRoute\":{},\n  \"initPath\":{},\n  \"qemu\":{},\n  \"serveDirectV86\":{{\n    \"argv\":{},\n    \"bundle\":\"direct-v86\",\n    \"wanixServices\":true,\n    \"p9Msize\":{}\n  }}\n}}\n",
        json_string(&out),
        json_string(&kernel_route),
        json_string(kernel_path.to_string_lossy().as_ref()),
        json_string(&init_route),
        json_string(init_path.to_string_lossy().as_ref()),
        qemu,
        json_string_array(serve_argv),
        DEFAULT_P9_MSIZE,
    ))
}
