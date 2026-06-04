use std::fs;
use std::path::{Path, PathBuf};

use crate::CliError;
use crate::json::{json_string, json_string_array};

use super::QemuCommand;

pub(crate) const DEFAULT_P9_MSIZE: u32 = 131_072;

const DEFAULT_KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];
const DEFAULT_INITRD_CANDIDATES: &[&str] =
    &["boot/initrd", "boot/initrd.img", "initrd", "initrd.img"];
const DEFAULT_INIT_PATH: &str = "bin/init";

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
    let root_path = canonical_existing_dir(&command.root_path, "qemu --root")?;
    let kernel_path = resolve_kernel_path(command, &root_path)?;
    let initrd_path = resolve_initrd_path(command, &root_path)?;
    let root = root_path.to_string_lossy();
    validate_qemu_option_fragment(&root, "qemu --root path")?;
    validate_default_init_path(command, &root_path)?;
    let cmdline = qemu_cmdline(command);

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
    ]);
    if let Some(initrd_path) = &initrd_path {
        argv.extend([
            "-initrd".to_owned(),
            initrd_path.to_string_lossy().into_owned(),
        ]);
    }
    argv.extend([
        "-append".to_owned(),
        cmdline.clone(),
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
    Ok(QemuHandoff {
        root_path,
        kernel_path,
        initrd_path,
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

fn resolve_initrd_path(
    command: &QemuCommand,
    root_path: &Path,
) -> Result<Option<PathBuf>, CliError> {
    if let Some(initrd_path) = &command.initrd_path {
        return canonical_existing_file(initrd_path, "qemu --initrd").map(Some);
    }
    for candidate in DEFAULT_INITRD_CANDIDATES {
        let path = root_path.join(candidate);
        if path.is_file() {
            return canonical_existing_file(&path, "qemu discovered initrd").map(Some);
        }
    }
    Ok(None)
}

fn validate_default_init_path(command: &QemuCommand, root_path: &Path) -> Result<(), CliError> {
    if command.cmdline.is_some() {
        return Ok(());
    }
    let init_path = root_path.join(DEFAULT_INIT_PATH);
    let metadata = fs::metadata(&init_path).map_err(|_| {
        CliError::new(
            format!(
                "qemu default cmdline expects /{DEFAULT_INIT_PATH} under {}; pass --cmdline TEXT \
                 to own init policy",
                root_path.display()
            ),
            1,
        )
    })?;
    if !metadata.is_file() {
        return Err(CliError::new(
            format!(
                "qemu default cmdline expects /{DEFAULT_INIT_PATH} under {}; pass --cmdline TEXT \
                 to own init policy",
                root_path.display()
            ),
            1,
        ));
    }
    validate_default_init_executable(&init_path, &metadata, root_path)
}

#[cfg(unix)]
fn validate_default_init_executable(
    init_path: &Path,
    metadata: &fs::Metadata,
    root_path: &Path,
) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o111 != 0 {
        return Ok(());
    }
    Err(CliError::new(
        format!(
            "qemu default cmdline expects /{DEFAULT_INIT_PATH} under {} to be executable; \
             pass --cmdline TEXT to own init policy ({})",
            root_path.display(),
            init_path.display()
        ),
        1,
    ))
}

#[cfg(not(unix))]
fn validate_default_init_executable(
    _init_path: &Path,
    _metadata: &fs::Metadata,
    _root_path: &Path,
) -> Result<(), CliError> {
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
