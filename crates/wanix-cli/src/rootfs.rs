use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use wanix_task::quote_cmd_argv;

use crate::{CliError, CliOutput};

const KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];
const INIT_PATH: &str = "bin/init";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootfsCommand {
    archive_path: PathBuf,
    out_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootfsReport {
    out_path: PathBuf,
    kernel_route: &'static str,
}

pub(super) fn parse_rootfs_command(args: &[OsString]) -> Result<RootfsCommand, CliError> {
    let mut archive_path = None;
    let mut out_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--archive" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("rootfs --archive expects FILE"))?;
            archive_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--out" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("rootfs --out expects DIR"))?;
            out_path = Some(PathBuf::from(value));
            i += 1;
        } else {
            return Err(CliError::usage(format!(
                "unknown rootfs option: {}",
                args[i].to_string_lossy()
            )));
        }
    }
    let archive_path =
        archive_path.ok_or_else(|| CliError::usage("rootfs requires --archive FILE"))?;
    let out_path = out_path.ok_or_else(|| CliError::usage("rootfs requires --out DIR"))?;
    Ok(RootfsCommand {
        archive_path,
        out_path,
    })
}

pub(super) fn run_rootfs_command(command: RootfsCommand) -> Result<CliOutput, CliError> {
    let report = prepare_rootfs(command)?;
    let out = report.out_path.to_string_lossy().into_owned();
    let qemu = quote_cmd_argv(["wanix-rust", "qemu", "--root", &out, "--exec"]);
    let serve = quote_cmd_argv([
        "wanix-rust",
        "serve",
        &out,
        "--bundle",
        "direct-v86",
        "--wanix-services",
    ]);
    let stdout = format!(
        "rootfs extracted to {}\n\
         kernel /{}\n\
         init /{}\n\
         qemu {qemu}\n\
         serve {serve}\n",
        report.out_path.display(),
        report.kernel_route,
        INIT_PATH,
    );
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

fn prepare_rootfs(command: RootfsCommand) -> Result<RootfsReport, CliError> {
    let archive_path = canonical_existing_file(&command.archive_path, "rootfs --archive")?;
    ensure_output_dir_ready(&command.out_path)?;
    extract_tgz(&archive_path, &command.out_path)?;
    let out_path = fs::canonicalize(&command.out_path).map_err(|error| {
        CliError::new(
            format!(
                "rootfs --out {} is not readable after extraction: {error}",
                command.out_path.display()
            ),
            1,
        )
    })?;
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
    if !init_path.is_file() {
        return Err(CliError::new(
            format!("rootfs {} is missing /{INIT_PATH}", out_path.display()),
            1,
        ));
    }
    Ok(RootfsReport {
        out_path,
        kernel_route,
    })
}

fn ensure_output_dir_ready(path: &Path) -> Result<(), CliError> {
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

fn extract_tgz(archive_path: &Path, out_path: &Path) -> Result<(), CliError> {
    let file = fs::File::open(archive_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open rootfs archive {}: {error}",
                archive_path.display()
            ),
            1,
        )
    })?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let entries = archive.entries().map_err(|error| {
        CliError::new(
            format!(
                "failed to read rootfs archive {}: {error}",
                archive_path.display()
            ),
            1,
        )
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| {
            CliError::new(
                format!(
                    "failed to read rootfs archive entry from {}: {error}",
                    archive_path.display()
                ),
                1,
            )
        })?;
        let path = entry
            .path()
            .map_err(|error| {
                CliError::new(
                    format!(
                        "failed to read rootfs archive entry path from {}: {error}",
                        archive_path.display()
                    ),
                    1,
                )
            })?
            .into_owned();
        validate_archive_path(&path)?;
        if !entry.unpack_in(out_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to unpack rootfs archive entry {}: {error}",
                    path.display()
                ),
                1,
            )
        })? {
            return Err(CliError::new(
                format!("unsafe rootfs archive path {}", path.display()),
                1,
            ));
        }
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> Result<(), CliError> {
    let mut has_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CliError::new(
                    format!("unsafe rootfs archive path {}", path.display()),
                    1,
                ));
            }
        }
    }
    if has_normal {
        Ok(())
    } else {
        Err(CliError::new("rootfs archive contains an empty path", 1))
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
