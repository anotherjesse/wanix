use std::ffi::OsString;

use super::{QjsRunOptions, options::CommonQjsOption};
use crate::CliError;

type CommonQjsOptionHandler = fn(&OsString, &str, &str, &mut QjsRunOptions) -> Result<(), CliError>;

pub(super) const COMMON_QJS_OPTION_HANDLERS: &[(CommonQjsOption, CommonQjsOptionHandler)] = &[
    (CommonQjsOption::Env, apply_env_option),
    (CommonQjsOption::Cwd, apply_cwd_option),
    (CommonQjsOption::Stdin, apply_stdin_option),
    (CommonQjsOption::StdinFile, apply_stdin_file_option),
    (CommonQjsOption::EventLoopMs, apply_event_loop_ms_option),
    (CommonQjsOption::ReadyIoTurns, apply_ready_io_turns_option),
    (
        CommonQjsOption::InterruptAfter,
        apply_interrupt_after_option,
    ),
    (
        CommonQjsOption::MemoryLimitBytes,
        apply_memory_limit_bytes_option,
    ),
    (CommonQjsOption::Mount, apply_mount_option),
];

fn apply_env_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.add_env(value, label)
}

fn apply_cwd_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_cwd(value, label)
}

fn apply_stdin_option(
    value: &OsString,
    command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_stdin_bytes(value, command, label)
}

fn apply_stdin_file_option(
    value: &OsString,
    command: &str,
    _label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_stdin_file(value, command)
}

fn apply_event_loop_ms_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_event_loop_ms(value, label)
}

fn apply_ready_io_turns_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_ready_io_turns(value, label)
}

fn apply_interrupt_after_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_interrupt_after(value, label)
}

fn apply_memory_limit_bytes_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.set_memory_limit_bytes(value, label)
}

fn apply_mount_option(
    value: &OsString,
    _command: &str,
    label: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    options.add_mount(value, label)
}
