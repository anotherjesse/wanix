use std::fs;
use std::path::{Path, PathBuf};

use crate::CliError;
use crate::qemu::qemu_validate_root_path_for_handoff;

use super::{INIT_PATH, RootfsCommand, RootfsOutputFormat, RootfsReport, archive};

const KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];
const INIT_EXECUTE_PERMISSION_BITS: u32 = 0o111;

pub(super) fn prepare_rootfs(command: RootfsCommand) -> Result<RootfsReport, CliError> {
    let archive_path = canonical_existing_file(&command.archive_path, "rootfs --archive")?;
    ensure_output_dir_ready(&command.out_path)?;
    let out_path = fs::canonicalize(&command.out_path).map_err(|error| {
        CliError::new(
            format!(
                "rootfs --out {} is not readable after preparation: {error}",
                command.out_path.display()
            ),
            1,
        )
    })?;
    if command.output_format == RootfsOutputFormat::Json {
        qemu_validate_root_path_for_handoff(&out_path)?;
    }
    archive::extract_tgz(&archive_path, &out_path)?;
    prepared_rootfs_report(out_path)
}

pub(super) fn prepared_rootfs_report(out_path: PathBuf) -> Result<RootfsReport, CliError> {
    let kernel_route = first_existing(&out_path, KERNEL_CANDIDATES).ok_or_else(|| {
        CliError::new(
            format!(
                "rootfs {} is missing a guest kernel (tried /{})",
                out_path.display(),
                KERNEL_CANDIDATES.join(", /")
            ),
            1,
        )
    })?;
    let init_path = out_path.join(INIT_PATH);
    let init_metadata = fs::metadata(&init_path).map_err(|_| {
        CliError::new(
            format!("rootfs {} is missing /{INIT_PATH}", out_path.display()),
            1,
        )
    })?;
    if !init_metadata.is_file() {
        return Err(CliError::new(
            format!("rootfs {} is missing /{INIT_PATH}", out_path.display()),
            1,
        ));
    }
    validate_init_executable(&init_path, &init_metadata, &out_path)?;
    Ok(RootfsReport {
        out_path,
        kernel_route,
    })
}

#[cfg(unix)]
fn validate_init_executable(
    init_path: &Path,
    metadata: &fs::Metadata,
    out_path: &Path,
) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & INIT_EXECUTE_PERMISSION_BITS != 0 {
        return Ok(());
    }
    Err(CliError::new(
        format!(
            "rootfs {} has non-executable /{INIT_PATH} ({})",
            out_path.display(),
            init_path.display()
        ),
        1,
    ))
}

#[cfg(not(unix))]
fn validate_init_executable(
    _init_path: &Path,
    _metadata: &fs::Metadata,
    _out_path: &Path,
) -> Result<(), CliError> {
    Ok(())
}

pub(super) fn ensure_output_dir_ready(path: &Path) -> Result<(), CliError> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_dir() => Err(CliError::new(
            format!(
                "rootfs --out {} exists but is not a directory",
                path.display()
            ),
            1,
        )),
        Ok(_) => {
            let mut entries = fs::read_dir(path).map_err(|error| {
                CliError::new(
                    format!("rootfs --out {} is not readable: {error}", path.display()),
                    1,
                )
            })?;
            if entries.next().is_some() {
                return Err(CliError::new(
                    format!("rootfs --out {} must be empty", path.display()),
                    1,
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)
            .map_err(|error| {
                CliError::new(
                    format!("failed to create rootfs --out {}: {error}", path.display()),
                    1,
                )
            }),
        Err(error) => Err(CliError::new(
            format!("rootfs --out {} is not readable: {error}", path.display()),
            1,
        )),
    }
}

fn canonical_existing_file(path: &Path, label: &str) -> Result<PathBuf, CliError> {
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

fn first_existing(root: &Path, candidates: &'static [&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|route| root.join(route).is_file())
}
