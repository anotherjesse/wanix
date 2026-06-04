use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use wanix_fs::NormalizedPath;

use super::{
    HostMount, QjsCommand, QjsSnapshotFileCommand, QjsStdin, os_arg_to_string,
    parse_duration_millis, parse_host_mount, parse_u32, parse_usize, set_qjs_stdin,
    validate_env_line,
};
use crate::CliError;

pub(super) struct QjsRunOptions {
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
    pub(super) fn new() -> Result<Self, CliError> {
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

    pub(super) fn into_command(self, script_path: PathBuf, args: Vec<String>) -> QjsCommand {
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

    pub(super) fn into_snapshot_command(
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

pub(super) enum QjsOptionParse {
    Consumed,
    Separator,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommonQjsOption {
    Env,
    Cwd,
    Stdin,
    StdinFile,
    EventLoopMs,
    ReadyIoTurns,
    InterruptAfter,
    MemoryLimitBytes,
    Mount,
    Separator,
}

impl CommonQjsOption {
    fn from_arg(arg: &OsString) -> Option<Self> {
        match arg.to_str()? {
            "--env" => Some(Self::Env),
            "--cwd" => Some(Self::Cwd),
            "--stdin" => Some(Self::Stdin),
            "--stdin-file" => Some(Self::StdinFile),
            "--event-loop-ms" => Some(Self::EventLoopMs),
            "--ready-io-turns" => Some(Self::ReadyIoTurns),
            "--interrupt-after" => Some(Self::InterruptAfter),
            "--memory-limit-bytes" => Some(Self::MemoryLimitBytes),
            "--mount" => Some(Self::Mount),
            "--" => Some(Self::Separator),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Env => "--env",
            Self::Cwd => "--cwd",
            Self::Stdin => "--stdin",
            Self::StdinFile => "--stdin-file",
            Self::EventLoopMs => "--event-loop-ms",
            Self::ReadyIoTurns => "--ready-io-turns",
            Self::InterruptAfter => "--interrupt-after",
            Self::MemoryLimitBytes => "--memory-limit-bytes",
            Self::Mount => "--mount",
            Self::Separator => "--",
        }
    }

    fn expected(self) -> &'static str {
        match self {
            Self::Env => "KEY=VALUE",
            Self::Cwd => "a Wanix path",
            Self::Stdin => "text",
            Self::StdinFile => "PATH or -",
            Self::EventLoopMs => "milliseconds",
            Self::ReadyIoTurns | Self::InterruptAfter => "a count",
            Self::MemoryLimitBytes => "a byte count",
            Self::Mount => "HOST=GUEST",
            Self::Separator => "",
        }
    }

    fn apply(
        self,
        value: &OsString,
        command: &str,
        options: &mut QjsRunOptions,
    ) -> Result<(), CliError> {
        match self {
            Self::Env => {
                let value = os_arg_to_string(value, &format!("{command} --env"))?;
                validate_env_line(&value, &format!("{command} --env"))?;
                options.env.push(value);
            }
            Self::Cwd => {
                options.cwd =
                    NormalizedPath::new(os_arg_to_string(value, &format!("{command} --cwd"))?)?;
            }
            Self::Stdin => {
                set_qjs_stdin(
                    &mut options.stdin,
                    QjsStdin::Bytes(
                        os_arg_to_string(value, &format!("{command} --stdin"))?.into_bytes(),
                    ),
                    command,
                )?;
            }
            Self::StdinFile => {
                let source = if value == "-" {
                    QjsStdin::Process
                } else {
                    QjsStdin::File(PathBuf::from(value))
                };
                set_qjs_stdin(&mut options.stdin, source, command)?;
            }
            Self::EventLoopMs => {
                options.event_loop_wait_budget =
                    parse_duration_millis(value, &format!("{command} --event-loop-ms"))?;
            }
            Self::ReadyIoTurns => {
                options.ready_io_turns =
                    parse_usize(value, &format!("{command} --ready-io-turns"))?;
            }
            Self::InterruptAfter => {
                options.interrupt_poll_budget =
                    Some(parse_usize(value, &format!("{command} --interrupt-after"))?);
            }
            Self::MemoryLimitBytes => {
                options.memory_limit_bytes = Some(parse_u32(
                    value,
                    &format!("{command} --memory-limit-bytes"),
                )?);
            }
            Self::Mount => {
                options.mounts.push(parse_host_mount(
                    &os_arg_to_string(value, &format!("{command} --mount"))?,
                    &format!("{command} --mount"),
                )?);
            }
            Self::Separator => {}
        }
        Ok(())
    }
}

pub(super) fn parse_common_qjs_option(
    args: &[OsString],
    index: &mut usize,
    command: &str,
    options: &mut QjsRunOptions,
) -> Result<QjsOptionParse, CliError> {
    let Some(option) = CommonQjsOption::from_arg(&args[*index]) else {
        return Ok(QjsOptionParse::Unknown);
    };
    if option == CommonQjsOption::Separator {
        *index += 1;
        return Ok(QjsOptionParse::Separator);
    }
    let value = qjs_option_value(args, index, command, option.name(), option.expected())?;
    option.apply(value, command, options)?;
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
