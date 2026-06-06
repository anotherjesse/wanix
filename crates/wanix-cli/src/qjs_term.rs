#[cfg(all(test, unix))]
use std::collections::VecDeque;
use std::io::{Read, Write};
#[cfg(all(test, unix))]
use std::sync::Arc;
#[cfg(all(test, unix))]
use std::sync::Mutex;

use super::{CliError, CliOutput};

const QJS_SHELL_SOURCE: &str = include_str!("../../../examples/qjs-term-shell-demo.js");
const QJS_SHELL_SCRIPT_SENTINEL: &str = "__wanix_qjs_shell.js";
const QJS_SHELL_READY_IO_TURNS: usize = 2;
const QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS: u64 = 20;

mod command;
mod post_eval;
mod process;
mod program_spec;
mod pump;
mod runtime;
mod session;
mod terminal;

use command::{PostEvalFeed, qjs_shell_command};
pub(super) use command::{
    QjsShellCommand, QjsTermCommand, parse_qjs_shell_command, parse_qjs_term_command,
};
use program_spec::QjsTermProgram;
use pump::ProcessEventSources;
use runtime::{QjsTermProgramIo, QjsTermProgramRequest, run_qjs_term_program_streaming};
pub(crate) use session::QjsShellSession;

pub(super) struct QjsShellStreamingIo<'a> {
    pub(super) process_stdin: &'a mut dyn Read,
    pub(super) process_stdout: &'a mut dyn Write,
    pub(super) process_stderr: &'a mut dyn Write,
}

pub(super) fn run_qjs_term(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_term_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_term_streaming(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        QjsTermProgramRequest::blocking(
            command.qjs,
            command.feed_after_eval,
            QjsTermProgram::HostScript,
        ),
        QjsTermProgramIo {
            process_stdin,
            process_stdout,
            process_stderr,
        },
    )
}

fn qjs_shell_feed_after_eval(raw: bool) -> Vec<PostEvalFeed> {
    vec![if raw {
        PostEvalFeed::RawBytesProcess
    } else {
        PostEvalFeed::LinesProcess
    }]
}

fn qjs_shell_program_request(
    command: QjsShellCommand,
    event_sources: ProcessEventSources,
) -> QjsTermProgramRequest {
    QjsTermProgramRequest {
        qjs_command: qjs_shell_command(command.qjs, command.raw),
        feed_after_eval: qjs_shell_feed_after_eval(command.raw),
        program: QjsTermProgram::BundledShell,
        event_sources,
    }
}

fn qjs_term_program_io<'a>(
    process_stdin: &'a mut dyn Read,
    process_stdout: &'a mut dyn Write,
    process_stderr: &'a mut dyn Write,
) -> QjsTermProgramIo<'a> {
    QjsTermProgramIo {
        process_stdin,
        process_stdout,
        process_stderr,
    }
}

pub(super) fn run_qjs_shell(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_shell_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_shell_streaming(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(command, ProcessEventSources::blocking()),
        qjs_term_program_io(process_stdin, process_stdout, process_stderr),
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_input_fd(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(command, ProcessEventSources::input_fd(input_fd)),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_terminal_fds(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(
            command,
            ProcessEventSources::terminal_fds(input_fd, terminal_size_fd),
        ),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(all(test, unix))]
pub(super) fn run_qjs_shell_streaming_with_resize_queue(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    resize_queue: Arc<Mutex<VecDeque<(u16, u16)>>>,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(
            command,
            ProcessEventSources::resize_queue(input_fd, resize_queue),
        ),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(test)]
mod tests;
