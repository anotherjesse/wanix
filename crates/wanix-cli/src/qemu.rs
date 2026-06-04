use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use wanix_task::quote_cmd_argv;

use crate::{CliError, CliOutput};

const DEFAULT_QEMU_BIN: &str = "qemu-system-i386";
const DEFAULT_MEMORY_MB: u32 = 512;
const DEFAULT_QEMU_9P_ROOT_CMDLINE: &str = "console=hvc0 init=/bin/init rw root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L,msize=131072 loglevel=3";
const DEFAULT_KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QemuCommand {
    root_path: PathBuf,
    kernel_path: Option<PathBuf>,
    qemu_bin: String,
    memory_mb: u32,
    kvm: bool,
    cmdline: Option<String>,
    append: Vec<String>,
    exec: bool,
}

pub(super) fn parse_qemu_command(args: &[OsString]) -> Result<QemuCommand, CliError> {
    let mut root_path = None;
    let mut kernel_path = None;
    let mut qemu_bin = DEFAULT_QEMU_BIN.to_owned();
    let mut memory_mb = DEFAULT_MEMORY_MB;
    let mut kvm = true;
    let mut cmdline = None;
    let mut append = Vec::new();
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
    Ok(QemuCommand {
        root_path,
        kernel_path,
        qemu_bin,
        memory_mb,
        kvm,
        cmdline,
        append,
        exec,
    })
}

pub(super) fn run_qemu_command(command: QemuCommand) -> Result<CliOutput, CliError> {
    if command.exec {
        return Err(CliError::usage(
            "qemu --exec requires live process IO; use the wanix-rust binary",
        ));
    }
    let argv = qemu_virtio9p_argv(&command)?;
    let mut output = quote_cmd_argv(argv.iter().map(String::as_str)).into_bytes();
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
    let argv = qemu_virtio9p_argv(&command)?;
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

fn qemu_virtio9p_argv(command: &QemuCommand) -> Result<Vec<String>, CliError> {
    if command.memory_mb == 0 {
        return Err(CliError::usage(
            "qemu --memory-mb expects a positive integer",
        ));
    }
    let root_path = canonical_existing_dir(&command.root_path, "qemu --root")?;
    let kernel_path = resolve_kernel_path(command, &root_path)?;
    let root = root_path.to_string_lossy();
    if root.contains(',') {
        return Err(CliError::usage(
            "qemu --root path cannot contain ',' because QEMU -fsdev uses comma-separated options",
        ));
    }

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
        kernel_path.to_string_lossy().into_owned(),
        "-append".to_owned(),
        qemu_cmdline(command),
        "-fsdev".to_owned(),
        format!("local,id=host9p,path={root},security_model=mapped-xattr"),
        "-device".to_owned(),
        "virtio-9p-pci,fsdev=host9p,mount_tag=host9p".to_owned(),
        "-device".to_owned(),
        "virtio-serial-pci".to_owned(),
        "-device".to_owned(),
        "virtconsole,chardev=con".to_owned(),
        "-chardev".to_owned(),
        "stdio,id=con".to_owned(),
        "-nographic".to_owned(),
    ]);
    Ok(argv)
}

fn resolve_kernel_path(command: &QemuCommand, root_path: &Path) -> Result<PathBuf, CliError> {
    if let Some(kernel_path) = &command.kernel_path {
        return canonical_existing_file(kernel_path, "qemu --kernel");
    }
    for candidate in DEFAULT_KERNEL_CANDIDATES {
        let path = root_path.join(candidate);
        if path.is_file() {
            return canonical_existing_file(&path, "qemu discovered kernel");
        }
    }
    Err(CliError::new(
        format!(
            "qemu could not find a guest kernel under {}; pass --kernel PATH \
             (tried {})",
            root_path.display(),
            DEFAULT_KERNEL_CANDIDATES.join(", ")
        ),
        1,
    ))
}

fn qemu_cmdline(command: &QemuCommand) -> String {
    let mut cmdline = command
        .cmdline
        .clone()
        .unwrap_or_else(|| DEFAULT_QEMU_9P_ROOT_CMDLINE.to_owned());
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

fn canonical_existing_dir(path: &PathBuf, label: &str) -> Result<PathBuf, CliError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        CliError::new(
            format!("{label} {} is not readable: {error}", path.display()),
            1,
        )
    })?;
    if !canonical.is_dir() {
        return Err(CliError::new(
            format!("{label} {} is not a directory", path.display()),
            1,
        ));
    }
    Ok(canonical)
}

fn canonical_existing_file(path: &PathBuf, label: &str) -> Result<PathBuf, CliError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        CliError::new(
            format!("{label} {} is not readable: {error}", path.display()),
            1,
        )
    })?;
    if !canonical.is_file() {
        return Err(CliError::new(
            format!("{label} {} is not a file", path.display()),
            1,
        ));
    }
    Ok(canonical)
}

fn parse_memory_mb(arg: &OsString) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, "qemu --memory-mb")?;
    let memory = value
        .parse::<u32>()
        .map_err(|_| CliError::usage("qemu --memory-mb expects a positive integer"))?;
    if memory == 0 {
        return Err(CliError::usage(
            "qemu --memory-mb expects a positive integer",
        ));
    }
    Ok(memory)
}

fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} expects UTF-8")))
}
