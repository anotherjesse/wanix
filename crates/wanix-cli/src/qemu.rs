use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use wanix_task::quote_cmd_argv;

use crate::{CliError, CliOutput};

mod handoff;
mod json;
mod parse;

pub(crate) use handoff::DEFAULT_P9_MSIZE;
use handoff::{qemu_virtio9p_handoff, validate_qemu_option_fragment};
use json::qemu_handoff_json;
pub(super) use parse::parse_qemu_command;

const DEFAULT_QEMU_BIN: &str = "qemu-system-i386";
const DEFAULT_MEMORY_MB: u32 = 512;
const DEFAULT_MOUNT_TAG: &str = "host9p";
const DEFAULT_SECURITY_MODEL: &str = "mapped-xattr";
const VALID_SECURITY_MODELS: &[&str] = &["mapped-xattr", "mapped-file", "passthrough", "none"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QemuCommand {
    root_path: PathBuf,
    kernel_path: Option<PathBuf>,
    initrd_path: Option<PathBuf>,
    qemu_bin: String,
    memory_mb: u32,
    kvm: bool,
    mount_tag: String,
    security_model: String,
    p9_msize: u32,
    cmdline: Option<String>,
    append: Vec<String>,
    output_format: QemuOutputFormat,
    exec: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QemuOutputFormat {
    Shell,
    Json,
}

pub(super) fn run_qemu_command(command: QemuCommand) -> Result<CliOutput, CliError> {
    if command.exec {
        return Err(CliError::usage(
            "qemu --exec requires live process IO; use the wanix-rust binary",
        ));
    }
    let handoff = qemu_virtio9p_handoff(&command)?;
    let output = match command.output_format {
        QemuOutputFormat::Shell => quote_cmd_argv(handoff.argv.iter().map(String::as_str)),
        QemuOutputFormat::Json => qemu_handoff_json(&handoff),
    };
    let mut output = output.into_bytes();
    output.push(b'\n');
    Ok(CliOutput::new(output, Vec::new(), 0))
}

pub(super) fn run_qemu_streaming(
    command: QemuCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    if !command.exec {
        return Ok(run_qemu_command(command)?.exit_code());
    }
    let handoff = qemu_virtio9p_handoff(&command)?;
    let argv = handoff.argv;
    let quoted = quote_cmd_argv(argv.iter().map(String::as_str));
    writeln!(process_stderr, "wanix-rust qemu exec: {quoted}")
        .map_err(|error| CliError::new(format!("failed to write process stderr: {error}"), 1))?;
    process_stderr
        .flush()
        .map_err(|error| CliError::new(format!("failed to flush process stderr: {error}"), 1))?;

    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            CliError::new(
                format!("failed to start qemu executable {}: {error}", argv[0]),
                1,
            )
        })?;
    Ok(status.code().unwrap_or(1))
}

pub(super) fn qemu_command_exec(command: &QemuCommand) -> bool {
    command.exec
}

pub(crate) fn qemu_default_json_handoff_for_root(root_path: &Path) -> Result<String, CliError> {
    let command = QemuCommand {
        root_path: root_path.to_path_buf(),
        kernel_path: None,
        initrd_path: None,
        qemu_bin: DEFAULT_QEMU_BIN.to_owned(),
        memory_mb: DEFAULT_MEMORY_MB,
        kvm: true,
        mount_tag: DEFAULT_MOUNT_TAG.to_owned(),
        security_model: DEFAULT_SECURITY_MODEL.to_owned(),
        p9_msize: DEFAULT_P9_MSIZE,
        cmdline: None,
        append: Vec::new(),
        output_format: QemuOutputFormat::Json,
        exec: false,
    };
    let handoff = qemu_virtio9p_handoff(&command)?;
    Ok(qemu_handoff_json(&handoff))
}

pub(crate) fn qemu_validate_root_path_for_handoff(root_path: &Path) -> Result<(), CliError> {
    let root = root_path.to_string_lossy();
    validate_qemu_option_fragment(&root, "qemu --root path")
}
