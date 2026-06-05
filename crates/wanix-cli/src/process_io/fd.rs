use std::ffi::OsString;

use super::ProcessIo;
use crate::{
    CliError,
    qjs_term::{self, QjsShellStreamingIo},
};

#[cfg(unix)]
pub(crate) fn run_with_stdin_fd(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
) -> Result<i32, CliError> {
    run_with_qjs_shell_input(args, io, QjsShellInput::StdinFd(stdin_fd))
}

#[cfg(unix)]
pub(crate) fn run_with_terminal_fds(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
) -> Result<i32, CliError> {
    run_with_qjs_shell_input(
        args,
        io,
        QjsShellInput::TerminalFds {
            stdin_fd,
            terminal_size_fd,
        },
    )
}

#[cfg(all(unix, test))]
pub(crate) fn run_with_resize_queue(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
    resize_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(u16, u16)>>>,
) -> Result<i32, CliError> {
    run_with_qjs_shell_input(
        args,
        io,
        QjsShellInput::ResizeQueue {
            stdin_fd,
            resize_queue,
        },
    )
}

#[cfg(unix)]
enum QjsShellInput {
    StdinFd(libc::c_int),
    TerminalFds {
        stdin_fd: libc::c_int,
        terminal_size_fd: libc::c_int,
    },
    #[cfg(test)]
    ResizeQueue {
        stdin_fd: libc::c_int,
        resize_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(u16, u16)>>>,
    },
}

#[cfg(unix)]
fn run_with_qjs_shell_input(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    input: QjsShellInput,
) -> Result<i32, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return super::run_with_process_io_inner(args, io);
    };
    if command != "qjs-shell" {
        return super::run_with_process_io_inner(args, io);
    }

    let command = qjs_term::parse_qjs_shell_command(rest)?;
    match input {
        QjsShellInput::StdinFd(stdin_fd) => qjs_term::run_qjs_shell_streaming_with_input_fd(
            command,
            stdin_fd,
            qjs_shell_streaming_io(io),
        ),
        QjsShellInput::TerminalFds {
            stdin_fd,
            terminal_size_fd,
        } => qjs_term::run_qjs_shell_streaming_with_terminal_fds(
            command,
            stdin_fd,
            terminal_size_fd,
            qjs_shell_streaming_io(io),
        ),
        #[cfg(test)]
        QjsShellInput::ResizeQueue {
            stdin_fd,
            resize_queue,
        } => qjs_term::run_qjs_shell_streaming_with_resize_queue(
            command,
            stdin_fd,
            resize_queue,
            qjs_shell_streaming_io(io),
        ),
    }
}

#[cfg(unix)]
fn qjs_shell_streaming_io<'a>(io: &'a mut ProcessIo<'_>) -> QjsShellStreamingIo<'a> {
    QjsShellStreamingIo {
        process_stdin: io.stdin,
        process_stdout: io.stdout,
        process_stderr: io.stderr,
    }
}
