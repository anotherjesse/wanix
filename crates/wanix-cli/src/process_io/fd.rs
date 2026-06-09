use std::ffi::OsString;

use super::ProcessIo;
use crate::{
    CliError,
    qjs_term::{self, QjsShellStreamingIo},
    sh,
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

/// What the fd-aware entry runs: the two live terminal sessions (`qjs-shell`
/// and interactive `sh`), or the ordinary process-IO path for everything else
/// (including `sh -c`, which is a captured one-line run).
#[cfg(unix)]
enum TerminalSessionRoute {
    Passthrough,
    QjsShell(qjs_term::QjsShellCommand),
    Sh(sh::ShCommand),
}

#[cfg(unix)]
fn run_with_qjs_shell_input(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    input: QjsShellInput,
) -> Result<i32, CliError> {
    match terminal_session_route(&args)? {
        TerminalSessionRoute::Passthrough => super::run_with_process_io_inner(args, io),
        TerminalSessionRoute::Sh(command) => {
            let (stdin_fd, terminal_size_fd) = input_fds(input);
            sh::run_sh_session(command, stdin_fd, terminal_size_fd, io.stdin, io.stdout)
        }
        TerminalSessionRoute::QjsShell(command) => run_qjs_shell_with_input(command, io, input),
    }
}

#[cfg(unix)]
fn terminal_session_route(args: &[OsString]) -> Result<TerminalSessionRoute, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(TerminalSessionRoute::Passthrough);
    };
    // `qjs-shell --help`/`sh --help` belong to the help path, not a live shell.
    if crate::help::wants_help(rest) {
        return Ok(TerminalSessionRoute::Passthrough);
    }
    if command == "qjs-shell" {
        return Ok(TerminalSessionRoute::QjsShell(
            qjs_term::parse_qjs_shell_command(rest)?,
        ));
    }
    if command == "sh" {
        let parsed = sh::parse_sh_command(rest)?;
        if parsed.line.is_none() {
            return Ok(TerminalSessionRoute::Sh(parsed));
        }
    }
    Ok(TerminalSessionRoute::Passthrough)
}

#[cfg(unix)]
fn input_fds(input: QjsShellInput) -> (libc::c_int, Option<libc::c_int>) {
    match input {
        QjsShellInput::StdinFd(stdin_fd) => (stdin_fd, None),
        QjsShellInput::TerminalFds {
            stdin_fd,
            terminal_size_fd,
        } => (stdin_fd, Some(terminal_size_fd)),
        #[cfg(test)]
        QjsShellInput::ResizeQueue { stdin_fd, .. } => (stdin_fd, None),
    }
}

#[cfg(unix)]
fn run_qjs_shell_with_input(
    command: qjs_term::QjsShellCommand,
    io: &mut ProcessIo<'_>,
    input: QjsShellInput,
) -> Result<i32, CliError> {
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
