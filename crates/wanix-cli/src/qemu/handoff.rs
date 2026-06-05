use std::path::PathBuf;

use crate::CliError;

use super::QemuCommand;

mod argv;
mod paths;

use argv::qemu_virtio9p_argv;
use paths::{QemuHandoffPaths, resolve_qemu_handoff_paths};

pub(crate) const DEFAULT_P9_MSIZE: u32 = 131_072;

pub(super) struct QemuHandoff {
    pub(super) root_path: PathBuf,
    pub(super) kernel_path: PathBuf,
    pub(super) initrd_path: Option<PathBuf>,
    pub(super) qemu_bin: String,
    pub(super) memory_mb: u32,
    pub(super) kvm: bool,
    pub(super) mount_tag: String,
    pub(super) security_model: String,
    pub(super) p9_msize: u32,
    pub(super) cmdline: String,
    pub(super) argv: Vec<String>,
}

pub(super) fn qemu_virtio9p_handoff(command: &QemuCommand) -> Result<QemuHandoff, CliError> {
    validate_qemu_handoff_sizes(command)?;
    let paths = resolve_qemu_handoff_paths(command)?;
    let cmdline = qemu_cmdline(command);
    let argv = qemu_virtio9p_argv(command, &paths, &cmdline);

    Ok(QemuHandoff {
        root_path: paths.root_path,
        kernel_path: paths.kernel_path,
        initrd_path: paths.initrd_path,
        qemu_bin: command.qemu_bin.clone(),
        memory_mb: command.memory_mb,
        kvm: command.kvm,
        mount_tag: command.mount_tag.clone(),
        security_model: command.security_model.clone(),
        p9_msize: command.p9_msize,
        cmdline,
        argv,
    })
}

fn validate_qemu_handoff_sizes(command: &QemuCommand) -> Result<(), CliError> {
    if command.memory_mb == 0 {
        return Err(CliError::usage(
            "qemu --memory-mb expects a positive integer",
        ));
    }
    if command.p9_msize == 0 {
        return Err(CliError::usage(
            "qemu --p9-msize expects a positive integer",
        ));
    }
    Ok(())
}

fn qemu_cmdline(command: &QemuCommand) -> String {
    let mut cmdline = command
        .cmdline
        .clone()
        .unwrap_or_else(|| default_qemu_9p_root_cmdline(&command.mount_tag, command.p9_msize));
    for append in &command.append {
        if append.is_empty() {
            continue;
        }
        if !cmdline.is_empty() {
            cmdline.push(' ');
        }
        cmdline.push_str(append);
    }
    cmdline
}

fn default_qemu_9p_root_cmdline(mount_tag: &str, p9_msize: u32) -> String {
    format!(
        "console=hvc0 init=/bin/init rw root={mount_tag} rootfstype=9p \
         rootflags=trans=virtio,version=9p2000.L,msize={p9_msize} loglevel=3"
    )
}

pub(super) fn validate_qemu_option_fragment(value: &str, label: &str) -> Result<(), CliError> {
    if value.is_empty() {
        return Err(CliError::usage(format!("{label} cannot be empty")));
    }
    if value.contains(',') {
        return Err(CliError::usage(format!(
            "{label} cannot contain ',' because QEMU device options are comma-separated"
        )));
    }
    Ok(())
}
