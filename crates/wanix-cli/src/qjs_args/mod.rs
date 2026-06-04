use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use wanix_fs::NormalizedPath;

use crate::CliError;

mod command;
mod restore;
mod snapshot;

pub(super) use command::{parse_qjs_command, parse_qjs_command_for};
pub(super) use restore::parse_qjs_restore_command;
pub(super) use snapshot::parse_qjs_snapshot_file_command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsCommand {
    pub(super) script_path: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) stdin: Option<QjsStdin>,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) ready_io_turns: usize,
    pub(super) interrupt_poll_budget: Option<usize>,
    pub(super) memory_limit_bytes: Option<u32>,
    pub(super) mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QjsStdin {
    Bytes(Vec<u8>),
    File(PathBuf),
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostMount {
    pub(super) host_path: PathBuf,
    pub(super) guest_path: NormalizedPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsRestoreCommand {
    pub(super) before_script_path: PathBuf,
    pub(super) after_script_path: PathBuf,
    pub(super) before_args: Vec<String>,
    pub(super) after_args: Vec<String>,
    pub(super) before_env: Vec<String>,
    pub(super) after_env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsSnapshotFileCommand {
    pub(super) script_path: PathBuf,
    pub(super) snapshot_path: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) stdin: Option<QjsStdin>,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) ready_io_turns: usize,
    pub(super) interrupt_poll_budget: Option<usize>,
    pub(super) memory_limit_bytes: Option<u32>,
    pub(super) mounts: Vec<HostMount>,
}

struct QjsRunOptions {
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
    memory_limit_bytes: Option<u32>,
    mounts: Vec<HostMount>,
}

impl QjsRunOptions {
    fn new() -> Result<Self, CliError> {
        Ok(Self {
            env: Vec::new(),
            cwd: NormalizedPath::new(".")?,
            stdin: None,
            event_loop_wait_budget: Duration::ZERO,
            ready_io_turns: 1,
            interrupt_poll_budget: None,
            memory_limit_bytes: None,
            mounts: Vec::new(),
        })
    }

    fn into_command(self, script_path: PathBuf, args: Vec<String>) -> QjsCommand {
        QjsCommand {
            script_path,
            args,
            env: self.env,
            cwd: self.cwd,
            stdin: self.stdin,
            event_loop_wait_budget: self.event_loop_wait_budget,
            ready_io_turns: self.ready_io_turns,
            interrupt_poll_budget: self.interrupt_poll_budget,
            memory_limit_bytes: self.memory_limit_bytes,
            mounts: self.mounts,
        }
    }

    fn into_snapshot_command(
        self,
        script_path: PathBuf,
        snapshot_path: PathBuf,
        args: Vec<String>,
    ) -> QjsSnapshotFileCommand {
        QjsSnapshotFileCommand {
            script_path,
            snapshot_path,
            args,
            env: self.env,
            cwd: self.cwd,
            stdin: self.stdin,
            event_loop_wait_budget: self.event_loop_wait_budget,
            ready_io_turns: self.ready_io_turns,
            interrupt_poll_budget: self.interrupt_poll_budget,
            memory_limit_bytes: self.memory_limit_bytes,
            mounts: self.mounts,
        }
    }
}

enum QjsOptionParse {
    Consumed,
    Separator,
    Unknown,
}

fn parse_common_qjs_option(
    args: &[OsString],
    index: &mut usize,
    command: &str,
    options: &mut QjsRunOptions,
) -> Result<QjsOptionParse, CliError> {
    if args[*index] == "--env" {
        let value = qjs_option_value(args, index, command, "--env", "KEY=VALUE")?;
        let value = os_arg_to_string(value, &format!("{command} --env"))?;
        validate_env_line(&value, &format!("{command} --env"))?;
        options.env.push(value);
    } else if args[*index] == "--cwd" {
        let value = qjs_option_value(args, index, command, "--cwd", "a Wanix path")?;
        options.cwd = NormalizedPath::new(os_arg_to_string(
            value,
            &format!("{command} --cwd"),
        )?)?;
    } else if args[*index] == "--stdin" {
        let value = qjs_option_value(args, index, command, "--stdin", "text")?;
        set_qjs_stdin(
            &mut options.stdin,
            QjsStdin::Bytes(os_arg_to_string(value, &format!("{command} --stdin"))?.into_bytes()),
            command,
        )?;
    } else if args[*index] == "--stdin-file" {
        let value = qjs_option_value(args, index, command, "--stdin-file", "PATH or -")?;
        let source = if value == "-" {
            QjsStdin::Process
        } else {
            QjsStdin::File(PathBuf::from(value))
        };
        set_qjs_stdin(&mut options.stdin, source, command)?;
    } else if args[*index] == "--event-loop-ms" {
        let value = qjs_option_value(args, index, command, "--event-loop-ms", "milliseconds")?;
        options.event_loop_wait_budget =
            parse_duration_millis(value, &format!("{command} --event-loop-ms"))?;
    } else if args[*index] == "--ready-io-turns" {
        let value = qjs_option_value(args, index, command, "--ready-io-turns", "a count")?;
        options.ready_io_turns = parse_usize(value, &format!("{command} --ready-io-turns"))?;
    } else if args[*index] == "--interrupt-after" {
        let value = qjs_option_value(args, index, command, "--interrupt-after", "a count")?;
        options.interrupt_poll_budget =
            Some(parse_usize(value, &format!("{command} --interrupt-after"))?);
    } else if args[*index] == "--memory-limit-bytes" {
        let value = qjs_option_value(args, index, command, "--memory-limit-bytes", "a byte count")?;
        options.memory_limit_bytes =
            Some(parse_u32(value, &format!("{command} --memory-limit-bytes"))?);
    } else if args[*index] == "--mount" {
        let value = qjs_option_value(args, index, command, "--mount", "HOST=GUEST")?;
        options.mounts.push(parse_host_mount(
            &os_arg_to_string(value, &format!("{command} --mount"))?,
            &format!("{command} --mount"),
        )?);
    } else if args[*index] == "--" {
        *index += 1;
        return Ok(QjsOptionParse::Separator);
    } else {
        return Ok(QjsOptionParse::Unknown);
    }
    Ok(QjsOptionParse::Consumed)
}

fn qjs_option_value<'a>(
    args: &'a [OsString],
    index: &mut usize,
    command: &str,
    option: &str,
    expected: &str,
) -> Result<&'a OsString, CliError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| CliError::usage(format!("{command} {option} expects {expected}")))?;
    *index += 1;
    Ok(value)
}

pub(super) fn read_qjs_stdin(
    source: Option<QjsStdin>,
    process_stdin: &mut dyn Read,
) -> Result<Option<Vec<u8>>, CliError> {
    match source {
        None => Ok(None),
        Some(QjsStdin::Bytes(bytes)) => Ok(Some(bytes)),
        Some(QjsStdin::File(path)) => std::fs::read(&path).map(Some).map_err(|error| {
            CliError::new(
                format!("failed to read stdin file {}: {error}", path.display()),
                1,
            )
        }),
        Some(QjsStdin::Process) => {
            let mut bytes = Vec::new();
            process_stdin.read_to_end(&mut bytes).map_err(|error| {
                CliError::new(format!("failed to read process stdin: {error}"), 1)
            })?;
            Ok(Some(bytes))
        }
    }
}

fn set_qjs_stdin(
    stdin: &mut Option<QjsStdin>,
    source: QjsStdin,
    command: &str,
) -> Result<(), CliError> {
    if stdin.is_some() {
        return Err(CliError::usage(format!(
            "{command} accepts only one of --stdin or --stdin-file"
        )));
    }
    *stdin = Some(source);
    Ok(())
}

fn parse_host_mount(value: &str, label: &str) -> Result<HostMount, CliError> {
    let Some((host, guest)) = value.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    };
    if host.is_empty() || guest.is_empty() {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    }
    let guest_path = NormalizedPath::new(guest)?;
    if guest_path.as_str() == "." {
        return Err(CliError::usage(format!(
            "{label} guest path must not be . in this demo"
        )));
    }
    Ok(HostMount {
        host_path: PathBuf::from(host),
        guest_path,
    })
}

pub(super) fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} must be valid UTF-8")))
}

fn parse_duration_millis(arg: &OsString, label: &str) -> Result<Duration, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let millis = value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))?;
    Ok(Duration::from_millis(millis))
}

fn parse_usize(arg: &OsString, label: &str) -> Result<usize, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<usize>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))
}

fn parse_u32(arg: &OsString, label: &str) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{label} expects a 32-bit non-negative integer")))
}

fn validate_env_line(line: &str, label: &str) -> Result<(), CliError> {
    let Some((key, _value)) = line.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    };
    if key.is_empty() || key.chars().any(char::is_whitespace) || line.contains('\n') {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    }
    Ok(())
}
