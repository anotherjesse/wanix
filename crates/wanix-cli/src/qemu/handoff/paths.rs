use std::fs;
use std::path::{Path, PathBuf};

use crate::CliError;

use super::super::QemuCommand;
use super::validate_qemu_option_fragment;

const DEFAULT_KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];
const DEFAULT_INITRD_CANDIDATES: &[&str] =
    &["boot/initrd", "boot/initrd.img", "initrd", "initrd.img"];
const DEFAULT_INIT_PATH: &str = "bin/init";
const DEFAULT_INIT_EXECUTE_PERMISSION_BITS: u32 = 0o111;

pub(super) struct QemuHandoffPaths {
    pub(super) root_path: PathBuf,
    pub(super) kernel_path: PathBuf,
    pub(super) initrd_path: Option<PathBuf>,
}

pub(super) fn resolve_qemu_handoff_paths(
    command: &QemuCommand,
) -> Result<QemuHandoffPaths, CliError> {
    let root_path = canonical_existing_dir(&command.root_path, "qemu --root")?;
    let kernel_path = resolve_kernel_path(command, &root_path)?;
    let initrd_path = resolve_initrd_path(command, &root_path)?;
    let root = root_path.to_string_lossy();
    validate_qemu_option_fragment(&root, "qemu --root path")?;
    validate_default_init_path(command, &root_path)?;

    Ok(QemuHandoffPaths {
        root_path,
        kernel_path,
        initrd_path,
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

    if metadata.permissions().mode() & DEFAULT_INIT_EXECUTE_PERMISSION_BITS != 0 {
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
