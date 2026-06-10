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

/// What the fd-aware entry runs: the live terminal sessions (`qjs-shell`,
/// interactive `sh`, and `recipe run` of a run-less recipe, which is an
/// interactive `sh` over the recipe's mounts), or the ordinary process-IO path
/// for everything else (including `sh -c` and `recipe run` with a run line,
/// which are captured runs).
#[cfg(unix)]
enum TerminalSessionRoute {
    Passthrough,
    QjsShell(qjs_term::QjsShellCommand),
    Sh(sh::ShCommand),
    /// `sh -c LINE`: routing already parsed the command (and parsing resolves
    /// catalog names, emitting the once-per-name audit line), so the collected
    /// run reuses the parse instead of re-parsing — and re-resolving — it.
    ShCollected(sh::ShCommand),
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
        TerminalSessionRoute::ShCollected(command) => {
            let output = sh::run_sh(command, io.stdin)?;
            super::write_process_output(io.stdout, "stdout", output.stdout())?;
            super::write_process_output(io.stderr, "stderr", output.stderr())?;
            Ok(output.exit_code())
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
        return Ok(if parsed.line.is_none() {
            TerminalSessionRoute::Sh(parsed)
        } else {
            TerminalSessionRoute::ShCollected(parsed)
        });
    }
    if command == "recipe"
        && let Some(session) = crate::recipe::interactive_recipe_session(rest)?
    {
        return Ok(TerminalSessionRoute::Sh(session));
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

#[cfg(all(unix, test))]
mod tests {
    use std::ffi::OsString;

    use super::{TerminalSessionRoute, terminal_session_route};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// `sh -c` routing must reuse the parse it already made: parsing resolves
    /// catalog names (emitting the once-per-name stderr audit line), so a
    /// Passthrough re-parse would resolve — and log — every name twice.
    #[test]
    fn sh_with_a_line_routes_to_the_collected_run_without_reparsing() {
        let route = terminal_session_route(&args(&["sh", "-c", "echo hi"])).unwrap();
        match route {
            TerminalSessionRoute::ShCollected(command) => {
                assert_eq!(command.line.as_deref(), Some("echo hi"));
            }
            _ => panic!("sh -c must route to ShCollected, not re-parse via Passthrough"),
        }
    }

    #[test]
    fn sh_without_a_line_routes_to_the_interactive_session() {
        let route = terminal_session_route(&args(&["sh"])).unwrap();
        assert!(matches!(route, TerminalSessionRoute::Sh(_)));
    }
}
