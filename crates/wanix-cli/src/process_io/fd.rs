use std::ffi::OsString;
use std::io::{Read, Write};

use crate::{CliError, qjs_term};

#[cfg(unix)]
pub(crate) fn run_with_stdin_fd(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return super::run_with_process_io(args, process_stdin, process_stdout, process_stderr);
    };
    if command == "qjs-shell" {
        return qjs_term::run_qjs_shell_streaming_with_input_fd(
            qjs_term::parse_qjs_shell_command(rest)?,
            process_stdin,
            stdin_fd,
            process_stdout,
            process_stderr,
        );
    }
    super::run_with_process_io(args, process_stdin, process_stdout, process_stderr)
}

#[cfg(unix)]
pub(crate) fn run_with_terminal_fds(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return super::run_with_process_io(args, process_stdin, process_stdout, process_stderr);
    };
    if command == "qjs-shell" {
        return qjs_term::run_qjs_shell_streaming_with_terminal_fds(
            qjs_term::parse_qjs_shell_command(rest)?,
            process_stdin,
            stdin_fd,
            terminal_size_fd,
            process_stdout,
            process_stderr,
        );
    }
    super::run_with_process_io(args, process_stdin, process_stdout, process_stderr)
}

#[cfg(all(unix, test))]
pub(crate) fn run_with_resize_queue(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    resize_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(u16, u16)>>>,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return super::run_with_process_io(args, process_stdin, process_stdout, process_stderr);
    };
    if command == "qjs-shell" {
        return qjs_term::run_qjs_shell_streaming_with_resize_queue(
            qjs_term::parse_qjs_shell_command(rest)?,
            process_stdin,
            stdin_fd,
            resize_queue,
            process_stdout,
            process_stderr,
        );
    }
    super::run_with_process_io(args, process_stdin, process_stdout, process_stderr)
}
