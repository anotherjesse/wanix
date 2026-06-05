use super::super::QemuCommand;
use super::QemuHandoffPaths;

pub(super) fn qemu_virtio9p_argv(
    command: &QemuCommand,
    paths: &QemuHandoffPaths,
    cmdline: &str,
) -> Vec<String> {
    let root = paths.root_path.to_string_lossy();
    let mut argv = Vec::new();
    argv.push(command.qemu_bin.clone());
    if command.kvm {
        argv.extend([
            "-enable-kvm".to_owned(),
            "-cpu".to_owned(),
            "host".to_owned(),
        ]);
    }
    argv.extend([
        "-m".to_owned(),
        command.memory_mb.to_string(),
        "-smp".to_owned(),
        "1".to_owned(),
        "-kernel".to_owned(),
        paths.kernel_path.to_string_lossy().into_owned(),
    ]);
    if let Some(initrd_path) = &paths.initrd_path {
        argv.extend([
            "-initrd".to_owned(),
            initrd_path.to_string_lossy().into_owned(),
        ]);
    }
    argv.extend([
        "-append".to_owned(),
        cmdline.to_owned(),
        "-fsdev".to_owned(),
        format!(
            "local,id=host9p,path={root},security_model={}",
            command.security_model
        ),
        "-device".to_owned(),
        format!("virtio-9p-pci,fsdev=host9p,mount_tag={}", command.mount_tag),
        "-device".to_owned(),
        "virtio-serial-pci".to_owned(),
        "-device".to_owned(),
        "virtconsole,chardev=con".to_owned(),
        "-chardev".to_owned(),
        "stdio,id=con".to_owned(),
        "-nographic".to_owned(),
    ]);
    argv
}
