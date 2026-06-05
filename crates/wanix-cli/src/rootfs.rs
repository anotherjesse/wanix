use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

mod archive;
mod handoff;

use crate::qemu::qemu_validate_root_path_for_handoff;
use crate::{CliError, CliOutput};

const KERNEL_CANDIDATES: &[&str] = &["boot/bzImage", "bzImage"];
const INIT_PATH: &str = "bin/init";
const INIT_EXECUTE_PERMISSION_BITS: u32 = 0o111;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootfsCommand {
    archive_path: PathBuf,
    out_path: PathBuf,
    output_format: RootfsOutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootfsReport {
    out_path: PathBuf,
    kernel_route: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootfsOutputFormat {
    Text,
    Json,
}

pub(super) fn parse_rootfs_command(args: &[OsString]) -> Result<RootfsCommand, CliError> {
    let mut archive_path = None;
    let mut out_path = None;
    let mut output_format = RootfsOutputFormat::Text;
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
        } else if args[i] == "--json" {
            output_format = RootfsOutputFormat::Json;
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
        output_format,
    })
}

pub(super) fn run_rootfs_command(command: RootfsCommand) -> Result<CliOutput, CliError> {
    let output_format = command.output_format;
    let report = prepare_rootfs(command)?;
    let stdout = match output_format {
        RootfsOutputFormat::Text => handoff::rootfs_text_handoff(&report),
        RootfsOutputFormat::Json => handoff::rootfs_json_handoff(&report)?,
    };
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

pub(crate) fn rootfs_json_handoff_for_prepared_root(out_path: &Path) -> Result<String, CliError> {
    let out_path = fs::canonicalize(out_path).map_err(|error| {
        CliError::new(
            format!("rootfs {} is not readable: {error}", out_path.display()),
            1,
        )
    })?;
    qemu_validate_root_path_for_handoff(&out_path)?;
    handoff::rootfs_json_handoff(&prepared_rootfs_report(out_path)?)
}

fn prepare_rootfs(command: RootfsCommand) -> Result<RootfsReport, CliError> {
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

fn prepared_rootfs_report(out_path: PathBuf) -> Result<RootfsReport, CliError> {
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{RootfsOutputFormat, parse_rootfs_command};

    #[test]
    fn parse_rootfs_command_accepts_required_paths_and_json_mode() {
        let command = parse_rootfs_command(&os_args([
            "--archive",
            "fixtures/rootfs.tgz",
            "--out",
            "target/rootfs",
            "--json",
        ]))
        .unwrap();

        assert_eq!(command.archive_path, PathBuf::from("fixtures/rootfs.tgz"));
        assert_eq!(command.out_path, PathBuf::from("target/rootfs"));
        assert_eq!(command.output_format, RootfsOutputFormat::Json);
    }

    #[test]
    fn parse_rootfs_command_defaults_to_text_output() {
        let command = parse_rootfs_command(&os_args([
            "--archive",
            "fixtures/rootfs.tgz",
            "--out",
            "target/rootfs",
        ]))
        .unwrap();

        assert_eq!(command.output_format, RootfsOutputFormat::Text);
    }

    #[test]
    fn parse_rootfs_command_reports_missing_required_options() {
        assert_usage_error(parse_rootfs_command(&[]), "rootfs requires --archive FILE");
        assert_usage_error(
            parse_rootfs_command(&os_args(["--archive", "rootfs.tgz"])),
            "rootfs requires --out DIR",
        );
        assert_usage_error(
            parse_rootfs_command(&os_args(["--out", "target/rootfs"])),
            "rootfs requires --archive FILE",
        );
    }

    #[test]
    fn parse_rootfs_command_reports_option_boundary_errors() {
        assert_usage_error(
            parse_rootfs_command(&os_args(["--archive"])),
            "rootfs --archive expects FILE",
        );
        assert_usage_error(
            parse_rootfs_command(&os_args(["--out"])),
            "rootfs --out expects DIR",
        );
        assert_usage_error(
            parse_rootfs_command(&os_args(["--unknown"])),
            "unknown rootfs option: --unknown",
        );
    }

    fn assert_usage_error<T: std::fmt::Debug>(result: Result<T, crate::CliError>, expected: &str) {
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains(expected));
    }

    fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }
}
