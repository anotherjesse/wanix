use crate::json::{json_string, json_string_array};

use super::handoff::QemuHandoff;

pub(super) fn qemu_handoff_json(handoff: &QemuHandoff) -> String {
    let initrd_path = handoff
        .initrd_path
        .as_ref()
        .map(|path| json_string(path.to_string_lossy().as_ref()))
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\n  \"kind\":\"wanix-qemu-virtio9p.v1\",\n  \"qemuBin\":{},\n  \"argv\":{},\n  \"rootPath\":{},\n  \"kernelPath\":{},\n  \"initrdPath\":{},\n  \"cmdline\":{},\n  \"memoryMb\":{},\n  \"kvm\":{},\n  \"mountTag\":{},\n  \"securityModel\":{},\n  \"p9Msize\":{},\n  \"console\":\"hvc0\",\n  \"rootFilesystem\":\"9p\"\n}}",
        json_string(&handoff.qemu_bin),
        json_string_array(&handoff.argv),
        json_string(handoff.root_path.to_string_lossy().as_ref()),
        json_string(handoff.kernel_path.to_string_lossy().as_ref()),
        initrd_path,
        json_string(&handoff.cmdline),
        handoff.memory_mb,
        if handoff.kvm { "true" } else { "false" },
        json_string(&handoff.mount_tag),
        json_string(&handoff.security_model),
        handoff.p9_msize,
    )
}
