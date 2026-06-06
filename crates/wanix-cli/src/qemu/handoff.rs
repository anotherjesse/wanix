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
    qemu_cmdline_fragments(command).join(" ")
}

fn qemu_cmdline_fragments(command: &QemuCommand) -> Vec<String> {
    std::iter::once(base_qemu_cmdline(command))
        .chain(
            command
                .append
                .iter()
                .filter(|append| !append.is_empty())
                .cloned(),
        )
        .filter(|fragment| !fragment.is_empty())
        .collect()
}

fn base_qemu_cmdline(command: &QemuCommand) -> String {
    command
        .cmdline
        .clone()
        .unwrap_or_else(|| default_qemu_9p_root_cmdline(&command.mount_tag, command.p9_msize))
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::{
        DEFAULT_MEMORY_MB, DEFAULT_MOUNT_TAG, DEFAULT_QEMU_BIN, DEFAULT_SECURITY_MODEL,
        QemuCommand, QemuOutputFormat,
    };
    use super::{DEFAULT_P9_MSIZE, qemu_cmdline};

    fn command_with_cmdline(cmdline: Option<&str>, append: &[&str]) -> QemuCommand {
        QemuCommand {
            root_path: PathBuf::from("/guest"),
            kernel_path: None,
            initrd_path: None,
            qemu_bin: DEFAULT_QEMU_BIN.to_owned(),
            memory_mb: DEFAULT_MEMORY_MB,
            kvm: true,
            mount_tag: DEFAULT_MOUNT_TAG.to_owned(),
            security_model: DEFAULT_SECURITY_MODEL.to_owned(),
            p9_msize: DEFAULT_P9_MSIZE,
            cmdline: cmdline.map(str::to_owned),
            append: append.iter().map(|value| (*value).to_owned()).collect(),
            output_format: QemuOutputFormat::Shell,
            exec: false,
        }
    }

    #[test]
    fn qemu_cmdline_uses_default_9p_root_when_unspecified() {
        let command = command_with_cmdline(None, &[]);

        assert_eq!(
            qemu_cmdline(&command),
            "console=hvc0 init=/bin/init rw root=host9p rootfstype=9p \
             rootflags=trans=virtio,version=9p2000.L,msize=131072 loglevel=3"
        );
    }

    #[test]
    fn qemu_cmdline_appends_non_empty_fragments_in_order() {
        let command = command_with_cmdline(Some("console=hvc0"), &["", "single", "panic=1"]);

        assert_eq!(qemu_cmdline(&command), "console=hvc0 single panic=1");
    }

    #[test]
    fn qemu_cmdline_allows_append_to_supply_empty_custom_cmdline() {
        let command = command_with_cmdline(Some(""), &["", "single"]);

        assert_eq!(qemu_cmdline(&command), "single");
    }
}
