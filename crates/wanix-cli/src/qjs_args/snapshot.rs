use std::ffi::OsString;
use std::path::PathBuf;

use super::common::{QjsOptionParse, QjsRunOptions, parse_common_qjs_option};
use super::{QjsSnapshotFileCommand, os_arg_to_string};
use crate::CliError;

pub(crate) fn parse_qjs_snapshot_file_command(
    args: &[OsString],
    command: &str,
) -> Result<QjsSnapshotFileCommand, CliError> {
    let mut options = QjsRunOptions::new()?;
    let mut snapshot_path = None;
    let mut i = 0;
    while i < args.len() {
        match parse_common_qjs_option(args, &mut i, command, &mut options)? {
            QjsOptionParse::Consumed => continue,
            QjsOptionParse::Separator => break,
            QjsOptionParse::Unknown => {}
        }

        if args[i] == "--snapshot" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --snapshot expects FILE")))?;
            if snapshot_path.is_some() {
                return Err(CliError::usage(format!(
                    "{command} accepts only one --snapshot"
                )));
            }
            snapshot_path = Some(PathBuf::from(value));
            i += 1;
        } else {
            break;
        }
    }

    let snapshot_path = snapshot_path
        .ok_or_else(|| CliError::usage(format!("{command} requires --snapshot FILE")))?;
    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage(format!("{command} expects a script path")))?;
    let script_path = PathBuf::from(script);
    i += 1;

    if args.get(i).is_some_and(|arg| arg == "--") {
        i += 1;
    }

    let js_args = args[i..]
        .iter()
        .map(|arg| os_arg_to_string(arg, &format!("{command} script arg")))
        .collect::<Result<Vec<_>, CliError>>()?;

    Ok(options.into_snapshot_command(script_path, snapshot_path, js_args))
}
