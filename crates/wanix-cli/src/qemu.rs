use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use wanix_task::quote_cmd_argv;

use crate::{CliError, CliOutput};

mod handoff;

use handoff::{
    DEFAULT_P9_MSIZE, qemu_handoff_json, qemu_virtio9p_handoff, validate_qemu_option_fragment,
};

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

pub(super) fn parse_qemu_command(args: &[OsString]) -> Result<QemuCommand, CliError> {
    let mut root_path = None;
    let mut kernel_path = None;
    let mut initrd_path = None;
    let mut qemu_bin = DEFAULT_QEMU_BIN.to_owned();
    let mut memory_mb = DEFAULT_MEMORY_MB;
    let mut kvm = true;
    let mut mount_tag = DEFAULT_MOUNT_TAG.to_owned();
    let mut security_model = DEFAULT_SECURITY_MODEL.to_owned();
    let mut p9_msize = DEFAULT_P9_MSIZE;
    let mut cmdline = None;
    let mut append = Vec::new();
    let mut output_format = QemuOutputFormat::Shell;
    let mut exec = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --root expects DIR"))?;
            root_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--kernel" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --kernel expects PATH"))?;
            if kernel_path.is_some() {
                return Err(CliError::usage("qemu accepts only one --kernel"));
            }
            kernel_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--initrd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --initrd expects PATH"))?;
            if initrd_path.is_some() {
                return Err(CliError::usage("qemu accepts only one --initrd"));
            }
            initrd_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--cmdline" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --cmdline expects TEXT"))?;
            if cmdline.is_some() {
                return Err(CliError::usage("qemu accepts only one --cmdline"));
            }
            cmdline = Some(os_arg_to_string(value, "qemu --cmdline")?);
            i += 1;
        } else if args[i] == "--append" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --append expects TEXT"))?;
            append.push(os_arg_to_string(value, "qemu --append")?);
            i += 1;
        } else if args[i] == "--qemu-bin" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --qemu-bin expects PATH"))?;
            qemu_bin = os_arg_to_string(value, "qemu --qemu-bin")?;
            i += 1;
        } else if args[i] == "--memory-mb" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --memory-mb expects N"))?;
            memory_mb = parse_memory_mb(value)?;
            i += 1;
        } else if args[i] == "--p9-msize" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --p9-msize expects N"))?;
            p9_msize = parse_p9_msize(value)?;
            i += 1;
        } else if args[i] == "--mount-tag" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --mount-tag expects TAG"))?;
            mount_tag = parse_mount_tag(value)?;
            i += 1;
        } else if args[i] == "--security-model" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qemu --security-model expects MODEL"))?;
            security_model = parse_security_model(value)?;
            i += 1;
        } else if args[i] == "--json" {
            output_format = QemuOutputFormat::Json;
            i += 1;
        } else if args[i] == "--no-kvm" {
            kvm = false;
            i += 1;
        } else if args[i] == "--exec" {
            exec = true;
            i += 1;
        } else {
            return Err(CliError::usage(format!(
                "unknown qemu option: {}",
                args[i].to_string_lossy()
            )));
        }
    }
    let root_path = root_path.ok_or_else(|| CliError::usage("qemu requires --root DIR"))?;
    if exec && output_format == QemuOutputFormat::Json {
        return Err(CliError::usage(
            "qemu --json cannot be combined with --exec",
        ));
    }
    Ok(QemuCommand {
        root_path,
        kernel_path,
        initrd_path,
        qemu_bin,
        memory_mb,
        kvm,
        mount_tag,
        security_model,
        p9_msize,
        cmdline,
        append,
        output_format,
        exec,
    })
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

fn parse_memory_mb(arg: &OsString) -> Result<u32, CliError> {
    parse_positive_u32(arg, "qemu --memory-mb")
}

fn parse_p9_msize(arg: &OsString) -> Result<u32, CliError> {
    parse_positive_u32(arg, "qemu --p9-msize")
}

fn parse_positive_u32(arg: &OsString, label: &str) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let parsed = value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{label} expects a positive integer")))?;
    if parsed == 0 {
        return Err(CliError::usage(format!(
            "{label} expects a positive integer"
        )));
    }
    Ok(parsed)
}

fn parse_qemu_option_value(arg: &OsString, label: &str) -> Result<String, CliError> {
    let value = os_arg_to_string(arg, label)?;
    validate_qemu_option_fragment(&value, label)?;
    Ok(value)
}

fn parse_mount_tag(arg: &OsString) -> Result<String, CliError> {
    let value = parse_qemu_option_value(arg, "qemu --mount-tag")?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CliError::usage(
            "qemu --mount-tag accepts only ASCII letters, digits, '.', '_', and '-'",
        ));
    }
    Ok(value)
}

fn parse_security_model(arg: &OsString) -> Result<String, CliError> {
    let value = parse_qemu_option_value(arg, "qemu --security-model")?;
    if !VALID_SECURITY_MODELS.contains(&value.as_str()) {
        return Err(CliError::usage(format!(
            "qemu --security-model expects one of {}",
            VALID_SECURITY_MODELS.join(", ")
        )));
    }
    Ok(value)
}

fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} expects UTF-8")))
}
