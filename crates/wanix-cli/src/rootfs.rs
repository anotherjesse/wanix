use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

mod archive;
mod handoff;
mod prepare;
#[cfg(test)]
mod tests;

use crate::qemu::qemu_validate_root_path_for_handoff;
use crate::{CliError, CliOutput};

const INIT_PATH: &str = "bin/init";

#[cfg(test)]
use prepare::ensure_output_dir_ready;

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
    handoff::rootfs_json_handoff(&prepare::prepared_rootfs_report(out_path)?)
}

fn prepare_rootfs(command: RootfsCommand) -> Result<RootfsReport, CliError> {
    prepare::prepare_rootfs(command)
}
