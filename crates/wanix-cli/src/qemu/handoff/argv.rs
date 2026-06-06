use super::super::QemuCommand;
use super::QemuHandoffPaths;

pub(super) fn qemu_virtio9p_argv(
    command: &QemuCommand,
    paths: &QemuHandoffPaths,
    cmdline: &str,
) -> Vec<String> {
    let mut argv = Vec::new();
    argv.push(command.qemu_bin.clone());
    append_acceleration_args(&mut argv, command);
    append_machine_args(&mut argv, command, paths);
    append_initrd_arg(&mut argv, paths);
    append_root_filesystem_args(&mut argv, command, paths, cmdline);
    append_console_args(&mut argv);
    argv
}

fn append_acceleration_args(argv: &mut Vec<String>, command: &QemuCommand) {
    if command.kvm {
        argv.extend([
            "-enable-kvm".to_owned(),
            "-cpu".to_owned(),
            "host".to_owned(),
        ]);
    }
}

fn append_machine_args(argv: &mut Vec<String>, command: &QemuCommand, paths: &QemuHandoffPaths) {
    argv.extend([
        "-m".to_owned(),
        command.memory_mb.to_string(),
        "-smp".to_owned(),
        "1".to_owned(),
        "-kernel".to_owned(),
        paths.kernel_path.to_string_lossy().into_owned(),
    ]);
}

fn append_initrd_arg(argv: &mut Vec<String>, paths: &QemuHandoffPaths) {
    if let Some(initrd_path) = &paths.initrd_path {
        argv.extend([
            "-initrd".to_owned(),
            initrd_path.to_string_lossy().into_owned(),
        ]);
    }
}

fn append_root_filesystem_args(
    argv: &mut Vec<String>,
    command: &QemuCommand,
    paths: &QemuHandoffPaths,
    cmdline: &str,
) {
    let root = paths.root_path.to_string_lossy();
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
    ]);
}

fn append_console_args(argv: &mut Vec<String>) {
    argv.extend([
        "-device".to_owned(),
        "virtio-serial-pci".to_owned(),
        "-device".to_owned(),
        "virtconsole,chardev=con".to_owned(),
        "-chardev".to_owned(),
        "stdio,id=con".to_owned(),
        "-nographic".to_owned(),
    ]);
}
