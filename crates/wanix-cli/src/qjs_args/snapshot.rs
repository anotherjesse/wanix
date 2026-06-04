use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use wanix_fs::NormalizedPath;

use super::{
    QjsSnapshotFileCommand, QjsStdin, os_arg_to_string, parse_duration_millis, parse_host_mount,
    parse_u32, parse_usize, set_qjs_stdin, validate_env_line,
};
use crate::CliError;

pub(crate) fn parse_qjs_snapshot_file_command(
    args: &[OsString],
    command: &str,
) -> Result<QjsSnapshotFileCommand, CliError> {
    let mut env = Vec::new();
    let mut cwd = NormalizedPath::new(".")?;
    let mut mounts = Vec::new();
    let mut stdin = None;
    let mut event_loop_wait_budget = Duration::ZERO;
    let mut ready_io_turns = 1usize;
    let mut interrupt_poll_budget = None;
    let mut memory_limit_bytes = None;
    let mut snapshot_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --env expects KEY=VALUE")))?;
            let value = os_arg_to_string(value, &format!("{command} --env"))?;
            validate_env_line(&value, &format!("{command} --env"))?;
            env.push(value);
            i += 1;
        } else if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --cwd expects a Wanix path")))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, &format!("{command} --cwd"))?)?;
            i += 1;
        } else if args[i] == "--stdin" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --stdin expects text")))?;
            set_qjs_stdin(
                &mut stdin,
                QjsStdin::Bytes(
                    os_arg_to_string(value, &format!("{command} --stdin"))?.into_bytes(),
                ),
                command,
            )?;
            i += 1;
        } else if args[i] == "--stdin-file" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --stdin-file expects PATH or -"))
            })?;
            let source = if value == "-" {
                QjsStdin::Process
            } else {
                QjsStdin::File(PathBuf::from(value))
            };
            set_qjs_stdin(&mut stdin, source, command)?;
            i += 1;
        } else if args[i] == "--interrupt-after" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --interrupt-after expects a count"))
            })?;
            interrupt_poll_budget =
                Some(parse_usize(value, &format!("{command} --interrupt-after"))?);
            i += 1;
        } else if args[i] == "--event-loop-ms" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --event-loop-ms expects milliseconds"))
            })?;
            event_loop_wait_budget =
                parse_duration_millis(value, &format!("{command} --event-loop-ms"))?;
            i += 1;
        } else if args[i] == "--ready-io-turns" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --ready-io-turns expects a count"))
            })?;
            ready_io_turns = parse_usize(value, &format!("{command} --ready-io-turns"))?;
            i += 1;
        } else if args[i] == "--memory-limit-bytes" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!(
                    "{command} --memory-limit-bytes expects a byte count"
                ))
            })?;
            memory_limit_bytes = Some(parse_u32(
                value,
                &format!("{command} --memory-limit-bytes"),
            )?);
            i += 1;
        } else if args[i] == "--mount" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --mount expects HOST=GUEST")))?;
            mounts.push(parse_host_mount(
                &os_arg_to_string(value, &format!("{command} --mount"))?,
                &format!("{command} --mount"),
            )?);
            i += 1;
        } else if args[i] == "--snapshot" {
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
        } else if args[i] == "--" {
            i += 1;
            break;
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

    Ok(QjsSnapshotFileCommand {
        script_path,
        snapshot_path,
        args: js_args,
        env,
        cwd,
        stdin,
        event_loop_wait_budget,
        ready_io_turns,
        interrupt_poll_budget,
        memory_limit_bytes,
        mounts,
    })
}
