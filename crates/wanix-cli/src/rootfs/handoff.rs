use wanix_task::quote_cmd_argv;

use crate::CliError;
use crate::json::{json_string, json_string_array};
use crate::qemu::{DEFAULT_P9_MSIZE, qemu_default_json_handoff_for_root};

use super::{INIT_PATH, RootfsReport};

pub(super) fn rootfs_text_handoff(report: &RootfsReport) -> String {
    let out = report.out_path.to_string_lossy().into_owned();
    let qemu = quote_cmd_argv(["wanix", "qemu", "--root", &out, "--exec"]);
    let serve = quote_cmd_argv([
        "wanix",
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
        "wanix",
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{rootfs_json_handoff, rootfs_text_handoff};
    use crate::rootfs::RootfsReport;

    #[test]
    fn text_handoff_reports_root_routes_and_launch_commands() {
        let root = PathBuf::from("/tmp/wanix root");
        let report = RootfsReport {
            out_path: root.clone(),
            kernel_route: "boot/bzImage",
        };

        let handoff = rootfs_text_handoff(&report);

        assert!(handoff.contains("rootfs extracted to /tmp/wanix root"));
        assert!(handoff.contains("kernel /boot/bzImage"));
        assert!(handoff.contains("init /bin/init"));
        assert!(handoff.contains("qemu wanix qemu --root '/tmp/wanix root' --exec"));
        assert!(
            handoff.contains(
                "serve wanix serve '/tmp/wanix root' --bundle direct-v86 --wanix-services"
            ),
            "{handoff}"
        );
    }

    #[test]
    fn json_handoff_reports_root_paths_and_direct_v86_launch_contract() {
        let root = prepared_root("wanix-cli-rootfs-handoff-json");
        let report = RootfsReport {
            out_path: root.clone(),
            kernel_route: "boot/bzImage",
        };

        let handoff = rootfs_json_handoff(&report).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&handoff).unwrap();

        assert_eq!(manifest["kind"], "wanix-rootfs.v1");
        assert_eq!(manifest["rootPath"], root.to_string_lossy().as_ref());
        assert_eq!(manifest["kernelRoute"], "/boot/bzImage");
        assert_eq!(
            manifest["kernelPath"],
            root.join("boot/bzImage").to_string_lossy().as_ref()
        );
        assert_eq!(manifest["initRoute"], "/bin/init");
        assert_eq!(
            manifest["initPath"],
            root.join("bin/init").to_string_lossy().as_ref()
        );
        assert_eq!(manifest["serveDirectV86"]["bundle"], "direct-v86");
        assert_eq!(manifest["serveDirectV86"]["wanixServices"], true);
        assert_eq!(manifest["serveDirectV86"]["p9Msize"], 131_072);
        assert_eq!(
            manifest["serveDirectV86"]["argv"],
            serde_json::json!([
                "wanix",
                "serve",
                root.to_string_lossy(),
                "--bundle",
                "direct-v86",
                "--wanix-services"
            ])
        );
        assert_eq!(manifest["qemu"]["kind"], "wanix-qemu-virtio9p.v1");
        assert_eq!(
            manifest["qemu"]["rootPath"],
            root.to_string_lossy().as_ref()
        );
        assert_eq!(manifest["qemu"]["p9Msize"], 131_072);
    }

    fn prepared_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        write_executable(root.join("bin/init").as_path(), b"#!/bin/sh\n");
        fs::canonicalize(root).unwrap()
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &[u8]) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn write_executable(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
    }
}
